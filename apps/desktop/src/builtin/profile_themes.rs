//! Steam's profile themes (Profile Settings → Theme): their names and
//! colors, and the profile pages recolored to match.
//!
//! The colors are Steam's own, copied from the `body.<id>Theme` rules in
//! steamcommunity.com's profilev2.css (one row per theme, in that file's
//! order of variables), like a picture taken from Steam.

use egui::Color32;
use i18n::t;
use ui::Theme;
use ui::theme::{ColorRef, Fill};

/// Per theme: --gradient-right, --gradient-left, --gradient-background,
/// --gradient-background-right, --gradient-background-left,
/// --color-showcase-header, --gradient-showcase-header-left,
/// --btn-background, --btn-background-hover, --btn-outline.
const CSS: &[(&str, [&str; 10])] = &[
    ("Default", ["rgba(109, 38, 44, 0.301)", "rgba(50, 255, 193, 0.103)", "rgba(34, 35, 48, 0.93)", "rgba(109, 38, 44, 0)", "rgba(50, 255, 193, 0.103)", "rgba(43, 45, 68, 0.93)", "rgba(115, 173, 184, 0.247)", "rgba(43, 52, 68)", "rgb(53, 62, 78)", "rgb(93, 102, 118)"]),
    ("Cosmic", ["rgba(248, 70, 180, 0.301)", "rgba(9, 243, 99, 0.247)", "rgba(46, 13, 36, 0.93)", "rgba(70, 227, 248, 0)", "rgba(239, 243, 9, 0.13)", "rgba(57, 24, 61, 0.93)", "rgba(20, 60, 68, 0.93)", "rgb(90, 40, 92)", "rgb(100, 50, 102)", "rgba(140, 90, 142)"]),
    ("Summer", ["rgba(252, 197, 16, 0.301)", "rgba(9, 243, 153, 0.247)", "rgba(51, 27, 16, 0.93)", "rgba(252, 197, 16, 0)", "rgba(48, 243, 9, 0.13)", "rgba(70, 53, 31, 0.93)", "rgba(33, 78, 76, 0.93)", "rgb(95, 76, 39)", "rgb(105, 86, 49)", "rgb(145, 126, 89)"]),
    ("Midnight", ["rgba(51, 54, 253, 0.233)", "rgba(12, 85, 241, 0.37)", "rgba(10, 14, 32, 0.93)", "rgba(51, 54, 253, 0)", "rgba(12, 85, 241, 0.13)", "rgba(34, 32, 61, 0.93)", "rgba(20, 33, 68, 0.93)", "rgb(38, 36, 68)", "rgb(48, 46, 78)", "rgb(88, 86, 108)"]),
    ("DarkMode", ["rgba(49, 49, 49, 0.233)", "rgba(51, 51, 51, 0.37)", "rgba(24, 24, 24, 0.93)", "rgba(34, 34, 34, 0)", "rgba(32, 32, 32, 0.13)", "rgba(0, 0, 0, 0.5)", "rgba(0, 0, 0, 0.5)", "rgba(0, 0, 0, 0.5)", "rgba(40, 40, 40, 1)", "rgb(80, 80, 80)"]),
    ("Steel", ["rgba(70, 106, 128, 0.233)", "rgba(86, 120, 134, 0.37)", "rgba(41, 46, 51, 0.93)", "rgba(17, 24, 29, 0)", "rgba(19, 27, 31, 0)", "rgba(55, 62, 76, 0.93)", "rgba(68, 83, 93, 0.93)", "rgb(66, 76, 92)", "rgb(76, 86, 102)", "rgb(106, 126, 142)"]),
    ("PinkTeal", ["rgb(139 29 68)", "rgb(137 30 67)", "rgb(0 255 255 / 15%)", "rgb(0 0 0 / 0%)", "rgb(0 0 0 / 0%)", "rgb(42 117 122)", "rgb(42 117 122)", "rgb(42 117 122)", "rgb(57 166 173)", "rgb(55 162 169)"]),
    ("MutedRed", ["rgb(142 40 40 / 23%)", "rgb(163 53 53 / 37%)", "rgb(96 17 2 / 30%)", "rgb(253 51 51 / 0%)", "rgba(241, 12, 12, 0.13)", "rgb(49 9 9 / 81%)", "rgb(182 27 23 / 35%)", "rgb(58 13 10)", "rgb(105 26 21)", "rgb(108, 90, 86)"]),
    ("SteamGreen", ["#393e2b", "rgb(71 78 52)", "rgb(53 58 39)", "rgb(194 253 51 / 0%)", "rgb(206 241 12 / 0%)", "rgb(62 69 44)", "rgb(83 92 60)", "rgb(73 82 53)", "rgb(82 91 59)", "rgb(92 111 45)"]),
    ("BlueRed", ["rgb(253 56 51 / 23%)", "rgb(12 230 241 / 37%)", "rgb(10 25 32 / 30%)", "rgb(253 51 51 / 15%)", "rgb(12 241 241 / 13%)", "rgb(64 23 23 / 50%)", "rgb(27 113 132 / 93%)", "rgb(45 93 111)", "rgb(36 121 130)", "rgb(86 105 108)"]),
    ("GoldBurgundy", ["rgb(253 219 51 / 23%)", "rgb(97 75 37)", "rgb(39 27 16 / 50%)", "rgb(0 0 0 / 35%)", "rgb(0 0 0 / 35%)", "rgb(53 28 22 / 76%)", "rgb(120 92 20 / 69%)", "rgb(57 38 25)", "rgb(77 52 36)", "rgb(108, 90, 86)"]),
    ("VibrantBlue", ["#008ada8a", "rgb(47 137 188)", "#0203048a", "rgb(253 51 51 / 0%)", "rgb(12 140 241 / 17%)", "rgb(19 57 95)", "rgb(39 106 141)", "rgb(25 70 107)", "rgb(35 87 130)", "rgb(34 88 133)"]),
    ("GoldenProfileDebut", ["rgba(156, 66, 17, 0.18)", "rgba(213, 172, 81, 0.62)", "rgba(61, 47, 20, 0.6)", "rgba(0, 0, 0, 0)", "rgba(243, 200, 9, 0.13)", "rgba(95, 72, 33, 0.82)", "rgba(155, 122, 54, 0.9)", "rgb(125, 98, 44)", "rgb(135, 108, 54)", "rgb(175, 148, 94)"]),
    ("WinterProfile2020", ["#7770A988", "#34526F88", "#34526F88", "rgba(25, 20, 65, 0.333)", "rgba(3, 7, 12, 0.333)", "#07817f", "#34526F", "#34526F", "rgb(54, 104, 151)", "#34526F"]),
    ("GoldenWeekProfile2021", ["#389bb0aa", "#2c5e83aa", "#134061aa", "#389bb066", "#2c5e8333", "#389bb0", "#2c5e83", "#2c5e83", "#1b6daa", "#2c5e83"]),
    ("Summer2021", ["#344f68", "#ee3b57b8", "#000000cc", "#c1ad4c52", "#2e6ea2a3", "#b03849a1", "#2e6986", "#309186", "#3aa89c", "#2c5e83"]),
    ("MutedBlue", ["#737d96", "rgb(115 124 147)", "#2127338c", "rgb(253 51 51 / 0%)", "rgb(20 26 49 / 13%)", "rgb(30 44 66 / 54%)", "rgb(69 79 108 / 75%)", "rgb(44 55 74)", "rgb(63 81 111)", "rgb(86 91 108)"]),
    ("Gold", ["rgb(197 164 79 / 73%)", "rgb(128 94 36)", "rgb(116 93 44 / 64%)", "rgb(0 0 0 / 35%)", "rgb(0 0 0 / 35%)", "rgb(185 145 76 / 76%)", "rgb(120 95 31 / 69%)", "rgb(145 116 66)", "rgb(169 132 68)", "rgb(108, 90, 86)"]),
    ("BurntOrange", ["rgb(136 66 8 / 66%)", "rgb(251 250 251 / 48%)", "rgb(47 24 6 / 65%)", "rgb(19 8 2)", "rgb(8 2 5 / 0%)", "rgb(48 12 5 / 88%)", "rgb(168 99 5 / 67%)", "rgb(50 15 7)", "rgb(89 31 17)", "rgb(86 30 17)"]),
    ("FlatGrey", ["#3b3e4100", "rgb(59 62 65 / 0%)", "rgb(59 62 65)", "rgb(59 62 65 / 0%)", "rgb(59 62 65 / 0%)", "rgb(105 110 113)", "rgb(105 110 113)", "rgb(105 110 113)", "rgb(118 122 124)", "rgb(105 110 113)"]),
    ("Purple", ["#3c2c4c", "#52298094", "#20024380", "#0f021f", "#150329", "#4c2f69", "#29113f", "#40245c", "#5f3c80", "#603689"]),
    ("GreenSlime", ["rgb(254 249 156 / 30%)", "rgb(98 145 152 / 34%)", "rgb(35 58 26 / 58%)", "rgb(0 0 0)", "rgb(0 0 0)", "rgb(27 38 18)", "rgb(113 141 46)", "rgb(33 45 20)", "rgb(80 101 35)", "rgb(86 109 37)"]),
    ("Ghost", ["rgb(82 64 83 / 52%)", "rgb(3 3 4 / 46%)", "rgb(93 123 137 / 32%)", "rgb(72 31 46)", "rgb(0 0 0)", "rgb(69 32 46)", "rgb(93 123 136)", "rgb(93 122 135)", "rgb(112 151 169)", "rgb(126 126 126 / 27%)"]),
    ("ColorNightmare", ["rgb(58 183 180 / 51%)", "rgb(243 106 9 / 25%)", "rgb(0 0 0 / 50%)", "rgb(0 0 0)", "rgb(9 243 217 / 33%)", "rgb(57 144 152 / 61%)", "rgb(198 96 50)", "rgb(45 114 123)", "rgb(196 96 50)", "rgb(196 96 50)"]),
    ("Murugiah", ["rgb(84 75 158)", "rgb(84 75 158)", "rgb(84 75 158)", "rgb(84 75 158)", "rgb(84 75 158)", "rgb(11 160 232)", "rgb(11 160 232)", "rgb(225 69 40)", "rgb(145 181 69)", "rgb(225 69 40)"]),
    ("Winter2021", ["rgb(87 136 137 / 14%)", "rgb(157 204 199 / 35%)", "rgb(22 77 81 / 88%)", "rgb(95 165 155 / 15%)", "rgb(9 243 217 / 33%)", "rgb(184 99 69)", "rgb(207 132 36)", "rgb(184 99 69)", "rgb(206 131 36)", "rgb(196 96 50)"]),
    ("Lunar2022", ["rgb(232 63 12 / 78%)", "rgb(239 65 12)", "rgb(58 44 122 / 83%)", "rgb(135 25 2 / 65%)", "rgb(211 72 31 / 60%)", "rgb(86 62 173)", "rgb(234 101 69)", "rgb(86 62 173)", "rgb(146 78 131)", "rgb(173 85 113)"]),
    ("SteamDeck", ["transparent", "transparent", "transparent", "transparent", "transparent", "rgb(149 1 169 / 19%)", "rgb(87 89 123 / 0%)", "rgb(29 10 25 / 10%)", "rgb(175 52 149 / 21%)", "rgb(29 10 25 / 0%)"]),
    ("Steam3000", ["rgba(0, 0, 0, 0)", "rgba(0, 0, 0, 0)", "rgba(0, 0, 0, 0)", "rgba(0, 0, 0, 0)", "rgba(0, 0, 0, 0)", "rgb(231 196 52)", "rgb(239 0 255)", "rgba(0, 0, 0, 0)", "rgb(46, 46, 46)", "rgba(0, 0, 0, 0)"]),
];

/// One theme's colors, as Steam uses them on a profile.
#[derive(Clone, Copy)]
pub(crate) struct SteamTheme {
    /// Glows from the right and left (--gradient-right / --gradient-left).
    pub right: Color32,
    pub left: Color32,
    /// The profile's panel (--gradient-background).
    pub background: Color32,
    /// Where the glows fade to (--gradient-background-right / -left).
    pub background_right: Color32,
    pub background_left: Color32,
    /// Section header bars: from `header_left` to `header`.
    pub header: Color32,
    pub header_left: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub outline: Color32,
    /// The swatch's own color in Steam's theme picker (--edit-background).
    pub swatch: Color32,
}

/// --edit-background, which Steam's picker (community profile.css) sets
/// for these themes only; for the rest the swatch's base is black, as on
/// Steam.
const SWATCH: &[(&str, &str)] = &[("Default", "#2b3436"), ("Cosmic", "#5e2e6d"), ("Summer", "#815f1f"), ("DarkMode", "#161616"), ("Steel", "#424852")];

/// A CSS color as Steam writes them: `#hex`, `rgb()`/`rgba()` with commas
/// or spaces and an optional `/ 50%` alpha, or `transparent`.
fn css_color(s: &str) -> Color32 {
    let s = s.trim();
    if s == "transparent" {
        return Color32::TRANSPARENT;
    }
    if let Some(c) = ui::theme::parse_hex(s) {
        return c;
    }
    let inner = s.split_once('(').and_then(|(_, r)| r.strip_suffix(')')).unwrap_or("");
    let parts: Vec<&str> = inner.split([',', ' ', '/']).filter(|p| !p.is_empty()).collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.trim().parse::<f32>().ok()).unwrap_or(0.0);
    let alpha = match parts.get(3) {
        Some(a) if a.ends_with('%') => a.trim_end_matches('%').parse::<f32>().unwrap_or(100.0) / 100.0,
        Some(a) => a.parse::<f32>().unwrap_or(1.0),
        None => 1.0,
    };
    Color32::from_rgba_unmultiplied(num(0) as u8, num(1) as u8, num(2) as u8, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `top` laid over the solid `base`.
pub(crate) fn over(base: Color32, top: Color32) -> Color32 {
    let [r, g, b, a] = top.to_srgba_unmultiplied();
    let k = a as f32 / 255.0;
    let [br, bg, bb, _] = base.to_srgba_unmultiplied();
    let mix = |x: u8, y: u8| (y as f32 + (x as f32 - y as f32) * k).round() as u8;
    Color32::from_rgb(mix(r, br), mix(g, bg), mix(b, bb))
}

/// A radial glow as Steam's CSS and SVG draw them: `from` at the center
/// (solid out to `start` of the way), fading to `to` with no alpha at
/// `radius`.
#[derive(Clone, Copy)]
struct Glow {
    center: egui::Pos2,
    radius: f32,
    start: f32,
    from: Color32,
    to: Color32,
}

impl Glow {
    fn at(&self, p: egui::Pos2) -> Color32 {
        let t = (p.distance(self.center) / self.radius.max(1.0)).clamp(0.0, 1.0);
        let f = ((t - self.start) / (1.0 - self.start).max(0.001)).clamp(0.0, 1.0);
        let [r0, g0, b0, a0] = self.from.to_srgba_unmultiplied();
        let [r1, g1, b1, _] = self.to.to_srgba_unmultiplied();
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
        Color32::from_rgba_unmultiplied(lerp(r0, r1), lerp(g0, g1), lerp(b0, b1), (a0 as f32 * (1.0 - f)).round() as u8)
    }
}

/// `top` over `below`, both premultiplied (egui's colors).
fn blend(below: Color32, top: Color32) -> Color32 {
    let k = 1.0 - top.a() as f32 / 255.0;
    let c = |x: u8, y: u8| (x as f32 + y as f32 * k).round().min(255.0) as u8;
    Color32::from_rgba_premultiplied(c(top.r(), below.r()), c(top.g(), below.g()), c(top.b(), below.b()), c(top.a(), below.a()))
}

/// The color at `p`: `base` with each glow over it.
fn shade(base: Color32, glows: &[Glow], p: egui::Pos2) -> Color32 {
    glows.iter().fold(base, |c, g| blend(c, g.at(p)))
}

/// `rect` filled with `base` and the glows, as a fine mesh (each corner of
/// the grid takes its color, so the gradients come out smooth).
fn rect_mesh(rect: egui::Rect, base: Color32, glows: &[Glow]) -> egui::Mesh {
    let n = 24u32;
    let mut mesh = egui::Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let p = rect.min + egui::vec2(rect.width() * i as f32 / n as f32, rect.height() * j as f32 / n as f32);
            mesh.colored_vertex(p, shade(base, glows, p));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let k = j * (n + 1) + i;
            mesh.add_triangle(k, k + 1, k + n + 2);
            mesh.add_triangle(k, k + n + 2, k + n + 1);
        }
    }
    mesh
}

/// A circle filled the same way.
fn circle_mesh(center: egui::Pos2, radius: f32, base: Color32, glows: &[Glow]) -> egui::Mesh {
    let (rings, segs) = (10u32, 48u32);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, shade(base, glows, center));
    for r in 1..=rings {
        let rad = radius * r as f32 / rings as f32;
        for s in 0..segs {
            let a = std::f32::consts::TAU * s as f32 / segs as f32;
            let p = center + egui::vec2(a.cos(), a.sin()) * rad;
            mesh.colored_vertex(p, shade(base, glows, p));
        }
    }
    for s in 0..segs {
        mesh.add_triangle(0, 1 + s, 1 + (s + 1) % segs);
    }
    for r in 1..rings {
        let (inner, outer) = (1 + (r - 1) * segs, 1 + r * segs);
        for s in 0..segs {
            let n = (s + 1) % segs;
            mesh.add_triangle(inner + s, outer + s, outer + n);
            mesh.add_triangle(inner + s, outer + n, inner + n);
        }
    }
    mesh
}

impl SteamTheme {
    /// The overlay of Steam's theme picker (its swatches and preview are
    /// SVGs 382 or 401 wide): the panel color at 23%, a glow of the left
    /// color from the left edge and of the right color from the right,
    /// laid out in `box_` at that scale.
    fn picker_glows(&self, box_: egui::Rect, size: f32) -> ([Glow; 2], Color32) {
        let k = box_.width() / size;
        let at = |x: f32, y: f32| box_.min + egui::vec2(x, y) * k;
        let left = Glow { center: at(11.0, 126.0), radius: 187.0 * k, start: 0.0, from: self.left, to: self.background_left };
        let right = Glow { center: at(385.0, 148.0), radius: 320.0 * k, start: 0.349, from: self.right, to: self.background_right };
        ([left, right], self.background.gamma_multiply(0.23))
    }

    /// A swatch for the picker, as Steam draws it: its color, then the
    /// picker's overlay, in a circle.
    pub(crate) fn swatch_shape(&self, center: egui::Pos2, radius: f32) -> egui::Shape {
        let box_ = egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
        let (glows, tint) = self.picker_glows(box_, 382.0);
        let base = blend(self.swatch, tint);
        egui::Shape::mesh(circle_mesh(center, radius, base, &glows))
    }

    /// The picker's overlay over a preview of the profile in `rect` (drawn
    /// on top of the preview's own shapes).
    pub(crate) fn preview_overlay(&self, rect: egui::Rect) -> egui::Shape {
        let (glows, tint) = self.picker_glows(rect, 401.0);
        egui::Shape::mesh(rect_mesh(rect, tint, &glows))
    }

    /// A themed profile's panel in `rect`, from Steam's profile CSS: the
    /// panel color with glows from the top right (gone at 500 px), the top
    /// left (600 px), and the background tints from the bottom corners.
    pub(crate) fn panel_shape(&self, rect: egui::Rect, scale: f32) -> egui::Shape {
        let glow = |center: egui::Pos2, radius: f32, from: Color32| Glow { center, radius: radius * scale, start: 0.0, from, to: from.gamma_multiply(0.0) };
        let glows = [
            glow(rect.right_bottom(), 500.0, self.background_right),
            glow(rect.left_bottom(), 600.0, self.background_left),
            glow(rect.right_top(), 500.0, self.right),
            glow(rect.left_top(), 600.0, self.left),
        ];
        egui::Shape::mesh(rect_mesh(rect, self.background, &glows))
    }
}

/// A theme by its id ("Summer", "DarkMode"…); "" is Steam's default.
pub(crate) fn get(id: &str) -> Option<SteamTheme> {
    let id = if id.is_empty() { "Default" } else { id.trim_end_matches("Theme") };
    let (_, v) = CSS.iter().find(|(name, _)| name.eq_ignore_ascii_case(id))?;
    let c = |i: usize| css_color(v[i]);
    let swatch = SWATCH.iter().find(|(n, _)| n.eq_ignore_ascii_case(id)).map(|(_, h)| css_color(h)).unwrap_or(Color32::BLACK);
    Some(SteamTheme {
        right: c(0),
        left: c(1),
        background: c(2),
        background_right: c(3),
        background_left: c(4),
        header: c(5),
        header_left: c(6),
        button: c(7),
        button_hover: c(8),
        outline: c(9),
        swatch,
    })
}

/// A theme's name in your language. Steam sends keys like
/// "#ProfileTheme_Summer"; never show those.
pub(crate) fn name(id: &str, title: &str) -> String {
    let known = match id {
        "" | "Default" => Some(t!("Default Theme")),
        "Summer" => Some(t!("Summer")),
        "Midnight" => Some(t!("Midnight")),
        "Steel" => Some(t!("Steel")),
        "Cosmic" => Some(t!("Cosmic")),
        "DarkMode" => Some(t!("Dark Mode")),
        _ => None,
    };
    if let Some(n) = known {
        return n.to_string();
    }
    if !title.is_empty() && !title.starts_with('#') {
        return title.to_string();
    }
    // "GoldBurgundy" → "Gold Burgundy".
    let mut out = String::new();
    for (i, ch) in id.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// The app's theme with a Steam profile theme's colors on the profile
/// pages: the panel, the section boxes, rows, insets and buttons. `None`
/// for no theme (or one we don't know), so the app's look stays.
pub(crate) fn themed(t: &Theme, id: &str) -> Option<Theme> {
    if id.is_empty() {
        return None;
    }
    let s = get(id)?;
    let hex = |c: Color32| ColorRef(ui::theme::to_hex(c));
    let panel = over(t.chrome, s.background);
    let mut th = t.clone();
    let c = std::sync::Arc::make_mut(&mut th.components);
    // The panel itself is painted under the frame ([`panel_shape`]).
    c.profile_panel.normal.fill = Some(Fill::Color(ColorRef::from("transparent")));
    let half = |x: Color32| over(panel, x.gamma_multiply(0.5));
    c.profile_box.normal.fill = Some(Fill::Linear { linear: [hex(half(s.header_left)), hex(half(s.header))], horizontal: true, end: 1.0 });
    let button = over(panel, s.button);
    let hover = over(panel, s.button_hover);
    for comp in [&mut c.profile_inset, &mut c.profile_row] {
        comp.normal.fill = Some(Fill::Color(hex(button.gamma_multiply(0.45))));
    }
    c.profile_row.hover.fill = Some(Fill::Color(hex(hover)));
    c.profile_row.selected.fill = Some(Fill::Color(hex(hover)));
    c.button_secondary.normal.fill = Some(Fill::Color(hex(button)));
    c.button_secondary.hover.fill = Some(Fill::Color(hex(hover)));
    c.button_secondary.normal.border = Some(hex(over(panel, s.outline)));
    Some(th)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_steams_colors() {
        use egui::Color32;
        assert_eq!(super::css_color("rgb(95, 76, 39)"), Color32::from_rgb(95, 76, 39));
        assert_eq!(super::css_color("rgba(0, 0, 0, 0.5)").a(), 128);
        assert_eq!(super::css_color("rgb(0 255 255 / 15%)").to_srgba_unmultiplied(), [0, 255, 255, 38]);
        assert_eq!(super::css_color("#393e2b"), Color32::from_rgb(0x39, 0x3e, 0x2b));
        assert!(super::get("Summer").is_some() && super::get("DarkModeTheme").is_some() && super::get("").is_some());
        assert!(super::get("NoSuchTheme").is_none());
        assert_eq!(super::name("GoldBurgundy", "#ProfileTheme_GoldBurgundy"), "Gold Burgundy");
    }
}

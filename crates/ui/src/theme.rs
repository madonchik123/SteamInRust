//! Theme tokens, presets and typography.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    /// Window background.
    pub bg: Color32,
    /// Top bar, status bar, side panels.
    pub chrome: Color32,
    /// Cards.
    pub surface: Color32,
    pub surface_hover: Color32,
    /// Inputs, menus, raised elements.
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    /// Second stop of the accent gradient.
    pub accent_2: Color32,
    /// Text on accent fills.
    pub on_accent: Color32,
    pub play: Color32,
    pub online: Color32,
    pub in_game: Color32,
    pub away: Color32,
    pub danger: Color32,
    pub discount: Color32,
    pub radius: u8,
    /// Main call-to-action fill: primary buttons, the selected tab in a
    /// page's bar (the style guide's "Color / Secondary").
    pub secondary: Color32,
    /// The page bar under the top bar (the style guide's "Bg / Tertiary").
    pub tertiary: Color32,
    /// Store pages' background, top to bottom ([`Theme::paint_store_background`]).
    pub store_gradient: [Color32; 2],
    /// Library pages' background, center to edge ([`Theme::paint_library_background`]).
    pub library_gradient: [Color32; 2],
    /// Which tokens this theme was saved with. Themes saved before the
    /// redesign tokens existed read as 0 and get them filled in by
    /// [`Theme::upgrade`].
    #[serde(default)]
    pub rev: u8,
}

/// The current [`Theme::rev`].
pub const THEME_REV: u8 = 1;

impl Default for Theme {
    fn default() -> Self {
        Self::slate()
    }
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    pub fn presets() -> Vec<Theme> {
        vec![Self::slate(), Self::midnight(), Self::aurora(), Self::classic(), Self::oled(), Self::daylight()]
    }

    /// The default since the redesign: the Figma style guide's colors
    /// (frame `11:103`, "colors"). `docs/redesign.md` says what each is for.
    pub fn slate() -> Self {
        Self {
            name: "Slate".into(),
            dark: true,
            bg: hex(0x181D24),
            // Bg / Main.
            chrome: hex(0x0E141B),
            surface: hex(0x262C35),
            // Bg / Hover.
            surface_hover: hex(0x313843),
            // Bg / Highlight: menus, inputs, secondary buttons, tags.
            raised: hex(0x1E2329),
            border: hex(0x2A313B),
            // Text / Main, Text / Dim.
            text: hex(0xF3F3F3),
            text_dim: hex(0x76808C),
            text_faint: hex(0x525B66),
            // Color / Primary.
            accent: hex(0x66C0F4),
            accent_2: hex(0x4B619B),
            on_accent: hex(0x0E141B),
            // Accent / Green.
            play: hex(0xA1CD44),
            online: hex(0x66C0F4),
            in_game: hex(0xA1CD44),
            // Accent / Yellow, Accent / Red.
            away: hex(0xC1B15F),
            danger: hex(0xCD5444),
            discount: hex(0xA1CD44),
            radius: 5,
            // Color / Secondary, Bg / Tertiary.
            secondary: hex(0x4B619B),
            tertiary: hex(0x212B45),
            // --gpGradient-StoreBackground and --gpGradient-LibraryBackground.
            store_gradient: [hex(0x2A475E), hex(0x1B2838)],
            library_gradient: [hex(0x3D4450), hex(0x23262E)],
            rev: THEME_REV,
        }
    }

    /// Default: inky blue-black, electric blue to violet accent.
    pub fn midnight() -> Self {
        Self {
            name: "Midnight".into(),
            dark: true,
            bg: hex(0x0A0D12),
            chrome: hex(0x0E1218),
            surface: hex(0x131922),
            surface_hover: hex(0x1A222D),
            raised: hex(0x1C2530),
            border: hex(0x202935),
            text: hex(0xEEF2F7),
            text_dim: hex(0x9AA5B4),
            text_faint: hex(0x5F6B7A),
            accent: hex(0x4F8CFF),
            accent_2: hex(0x8B5CF6),
            on_accent: hex(0xFFFFFF),
            play: hex(0x2FB36B),
            online: hex(0x4FC3F7),
            in_game: hex(0x9CCC4B),
            away: hex(0xF2B84B),
            danger: hex(0xF0544F),
            discount: hex(0x4C6B22),
            radius: 12,
            secondary: hex(0x4F8CFF),
            tertiary: hex(0x111720),
            store_gradient: [hex(0x131922), hex(0x0A0D12)],
            library_gradient: [hex(0x1A222D), hex(0x0A0D12)],
            rev: THEME_REV,
        }
    }

    /// Deep teal with a mint-to-cyan accent.
    pub fn aurora() -> Self {
        Self {
            name: "Aurora".into(),
            bg: hex(0x081112),
            chrome: hex(0x0B1718),
            surface: hex(0x0F1E20),
            surface_hover: hex(0x15292B),
            raised: hex(0x172D30),
            border: hex(0x1B3336),
            text_dim: hex(0x93AFAE),
            text_faint: hex(0x58716F),
            accent: hex(0x2DD4BF),
            accent_2: hex(0x38BDF8),
            on_accent: hex(0x04201C),
            secondary: hex(0x1F9E8E),
            tertiary: hex(0x0D1A1C),
            store_gradient: [hex(0x0F1E20), hex(0x081112)],
            library_gradient: [hex(0x15292B), hex(0x081112)],
            ..Self::midnight()
        }
    }

    /// Familiar Steam-like blues.
    pub fn classic() -> Self {
        Self {
            name: "Classic".into(),
            bg: hex(0x1B2838),
            chrome: hex(0x171A21),
            surface: hex(0x213045),
            surface_hover: hex(0x2A3F5A),
            raised: hex(0x2A3B4F),
            border: hex(0x2E4058),
            text: hex(0xDCDEDF),
            text_dim: hex(0x8F98A0),
            text_faint: hex(0x5F6A75),
            accent: hex(0x66C0F4),
            accent_2: hex(0x417A9B),
            on_accent: hex(0x0E1C2A),
            play: hex(0x5C7E10),
            radius: 4,
            secondary: hex(0x417A9B),
            tertiary: hex(0x1F2C3D),
            store_gradient: [hex(0x2A475E), hex(0x1B2838)],
            library_gradient: [hex(0x3D4450), hex(0x23262E)],
            ..Self::midnight()
        }
    }

    /// True black for OLED screens.
    pub fn oled() -> Self {
        Self {
            name: "OLED".into(),
            bg: hex(0x000000),
            chrome: hex(0x050505),
            surface: hex(0x0E0E0F),
            surface_hover: hex(0x18181A),
            raised: hex(0x161618),
            border: hex(0x1F1F22),
            accent: hex(0xF5F5F5),
            accent_2: hex(0xA3A3A3),
            on_accent: hex(0x000000),
            secondary: hex(0x2A2A2E),
            tertiary: hex(0x0A0A0B),
            store_gradient: [hex(0x0E0E0F), hex(0x000000)],
            library_gradient: [hex(0x18181A), hex(0x000000)],
            ..Self::midnight()
        }
    }

    pub fn daylight() -> Self {
        Self {
            name: "Daylight".into(),
            dark: false,
            bg: hex(0xF3F5F8),
            chrome: hex(0xFFFFFF),
            surface: hex(0xFFFFFF),
            surface_hover: hex(0xEEF2F7),
            raised: hex(0xF7F9FB),
            border: hex(0xDEE3EA),
            text: hex(0x121A24),
            text_dim: hex(0x546273),
            text_faint: hex(0x8A97A6),
            accent: hex(0x2563EB),
            accent_2: hex(0x7C3AED),
            on_accent: hex(0xFFFFFF),
            play: hex(0x16A34A),
            online: hex(0x0284C7),
            in_game: hex(0x4D7C0F),
            away: hex(0xB45309),
            danger: hex(0xDC2626),
            discount: hex(0x4D7C0F),
            radius: 12,
            secondary: hex(0x2563EB),
            tertiary: hex(0xE8EDF3),
            store_gradient: [hex(0xFFFFFF), hex(0xF3F5F8)],
            library_gradient: [hex(0xFFFFFF), hex(0xEEF2F7)],
            rev: THEME_REV,
        }
    }

    /// Fill in tokens added after this theme was saved (see [`Theme::rev`])
    /// from the ones it has, so an older saved theme keeps its look.
    pub fn upgrade(&mut self) {
        if self.rev >= THEME_REV {
            return;
        }
        if let Some(p) = Self::presets().into_iter().find(|p| p.name == self.name) {
            // A preset: its own new tokens, but primary buttons follow an
            // accent color the user picked.
            self.secondary = if p.accent == self.accent { p.secondary } else { self.accent };
            self.tertiary = p.tertiary;
            self.store_gradient = p.store_gradient;
            self.library_gradient = p.library_gradient;
        } else {
            self.secondary = self.accent;
            self.tertiary = crate::widgets::mix(self.chrome, self.surface, 0.5);
            self.store_gradient = [self.surface, self.bg];
            self.library_gradient = [self.surface_hover, self.bg];
        }
        self.rev = THEME_REV;
    }

    /// Text that reads on `fill`: near-black on light fills (the Play
    /// button's green, the Install button's blue), near-white on dark ones.
    pub fn text_on(&self, fill: Color32) -> Color32 {
        let l = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
        if l > 140.0 { hex(0x0E141B) } else { hex(0xF3F3F3) }
    }

    /// The store's background (`--gpGradient-StoreBackground`): top to
    /// bottom, reaching the second color 80% of the way down.
    pub fn paint_store_background(&self, painter: &egui::Painter, rect: egui::Rect) {
        let [a, b] = self.store_gradient;
        let mid = egui::pos2(rect.right(), rect.top() + rect.height() * 0.8);
        crate::widgets::gradient(painter, egui::Rect::from_min_max(rect.min, mid), a, b, false);
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), mid.y), rect.max), egui::CornerRadius::ZERO, b);
    }

    /// The library's background (`--gpGradient-LibraryBackground`): a
    /// radial glow centered 40 px in from the top-left corner, reaching
    /// the second color 80% of the way to the farthest corner.
    pub fn paint_library_background(&self, painter: &egui::Painter, rect: egui::Rect) {
        let [a, b] = self.library_gradient;
        let center = rect.min + egui::vec2(40.0, 40.0);
        let far = (rect.max - center).length() * 0.8;
        crate::widgets::radial_gradient(painter, rect, center, far, a, b);
    }

    pub fn corner(&self) -> CornerRadius {
        CornerRadius::same(self.radius)
    }

    pub fn corner_small(&self) -> CornerRadius {
        CornerRadius::same((self.radius * 2 / 3).max(2))
    }

    /// Push these tokens into egui's global style.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        v.override_text_color = Some(self.text);
        v.panel_fill = self.bg;
        v.window_fill = self.raised;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = self.corner();
        v.window_shadow = egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(140) };
        // The style guide's menu shadow is (0, 4) blurred 4 at 25% black;
        // a little softer here so menus over pictures still stand out.
        v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(100) };
        v.menu_corner_radius = self.corner_small();
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
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(14.0, 7.0);
            s.spacing.interact_size.y = 32.0;
            s.spacing.menu_margin = egui::Margin::same(8);
            s.spacing.scroll.bar_width = 6.0;
            s.spacing.scroll.floating = true;
            s.spacing.scroll.bar_inner_margin = 2.0;
            s.interaction.selectable_labels = false;
            use egui::TextStyle::*;
            s.text_styles = [
                (Heading, FontId::new(type_scale::HEADING_M, bold())),
                (Body, FontId::new(type_scale::BODY_M, FontFamily::Proportional)),
                (Button, FontId::new(type_scale::BODY_M, medium())),
                (Small, FontId::new(type_scale::BODY_S, FontFamily::Proportional)),
                (Monospace, FontId::new(13.0, FontFamily::Monospace)),
            ]
            .into();
        });
    }
}

/// The style guide's type scale (frame `10:8`, "fonts / motiva sans").
/// The design is set in Motiva Sans, which is Valve's and can't be
/// shipped; Inter stands in at the same sizes and weights (headings bold,
/// body regular). Inter runs a little wider than Motiva at the same size.
pub mod type_scale {
    /// Heading Large: 26 px, bold.
    pub const HEADING_L: f32 = 26.0;
    /// Heading Medium: 22 px, bold.
    pub const HEADING_M: f32 = 22.0;
    /// Heading Small: 18 px, bold. Also the top bar's sections and large buttons.
    pub const HEADING_S: f32 = 18.0;
    /// Body Large: 16 px, regular. Page-bar tabs, secondary buttons, menu rows.
    pub const BODY_L: f32 = 16.0;
    /// Body Medium: 14 px, regular. The default text size.
    pub const BODY_M: f32 = 14.0;
    /// Body Small: 12 px, regular. Tags, review bars, captions.
    pub const BODY_S: f32 = 12.0;
}

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
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter", INTER_REGULAR),
        ("inter-medium", INTER_MEDIUM),
        ("inter-semibold", INTER_SEMIBOLD),
        ("inter-bold", INTER_BOLD),
        ("phosphor", PHOSPHOR),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    // System fonts for scripts Inter doesn't cover, so game names in Chinese,
    // Japanese, Korean, Thai, Arabic, Hindi… show instead of empty boxes.
    let mut system: Vec<String> = Vec::new();
    for path in system_font_files() {
        if let Some(bytes) = map_font(&path) {
            let name = format!("system-{}", path.file_name().unwrap_or_default().to_string_lossy());
            fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_static(bytes)));
            system.push(name);
        }
    }
    // Bold Chinese for headings and buttons (Windows ships it separately).
    let bold_cjk = std::env::var_os("WINDIR")
        .map(|w| std::path::PathBuf::from(w).join("Fonts").join("msyhbd.ttc"))
        .and_then(|p| map_font(&p))
        .map(|bytes| {
            fonts.font_data.insert("system-msyhbd".into(), Arc::new(egui::FontData::from_static(bytes)));
            "system-msyhbd".to_string()
        });
    // egui's bundled fonts stay as fallbacks for emoji and rare scripts.
    let mut fallbacks: Vec<String> = system;
    fallbacks.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    // The bundled font files are trimmed so they never overlap:
    // Inter has no private-use glyphs and Phosphor has only private-use ones.
    let family = |main: &str, heavy: bool| {
        let mut v = vec![main.to_string(), "phosphor".to_string()];
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
    fonts.families.entry(FontFamily::Monospace).or_default().push("phosphor".into());
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slate_is_the_default() {
        let t = Theme::default();
        assert_eq!(t.name, "Slate");
        assert_eq!(t.rev, THEME_REV);
        assert_eq!(Theme::presets()[0], t);
    }

    #[test]
    fn themes_saved_before_the_redesign_get_the_new_tokens() {
        // A Midnight theme saved before `secondary`, `tertiary` and the
        // gradients existed, with an accent the user picked.
        let mut old = serde_json::to_value(Theme::midnight()).unwrap();
        for key in ["secondary", "tertiary", "store_gradient", "library_gradient", "rev"] {
            old.as_object_mut().unwrap().remove(key);
        }
        old["accent"] = serde_json::to_value(Color32::from_rgb(1, 2, 3)).unwrap();
        let mut t: Theme = serde_json::from_value(old).unwrap();
        assert_eq!(t.rev, 0);
        t.upgrade();
        assert_eq!(t.rev, THEME_REV);
        // Primary buttons follow the picked accent; the rest is Midnight's.
        assert_eq!(t.secondary, Color32::from_rgb(1, 2, 3));
        assert_eq!(t.tertiary, Theme::midnight().tertiary);
        assert_eq!(t.library_gradient, Theme::midnight().library_gradient);
    }

    #[test]
    fn text_on_picks_a_readable_color() {
        let t = Theme::slate();
        assert_eq!(t.text_on(t.play), hex(0x0E141B));
        assert_eq!(t.text_on(t.secondary), hex(0xF3F3F3));
    }
}

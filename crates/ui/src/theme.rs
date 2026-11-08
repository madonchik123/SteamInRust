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
}

impl Default for Theme {
    fn default() -> Self {
        Self::midnight()
    }
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    pub fn presets() -> Vec<Theme> {
        vec![Self::midnight(), Self::aurora(), Self::classic(), Self::oled(), Self::daylight()]
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
        }
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
        v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) };
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
                (Heading, FontId::new(24.0, semibold())),
                (Body, FontId::new(14.0, FontFamily::Proportional)),
                (Button, FontId::new(14.0, medium())),
                (Small, FontId::new(12.0, FontFamily::Proportional)),
                (Monospace, FontId::new(13.0, FontFamily::Monospace)),
            ]
            .into();
        });
    }
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
    if let Some(dir) = std::env::var_os("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")) {
        for file in ["msyh.ttc", "YuGothM.ttc", "malgun.ttf", "Nirmala.ttc", "tahoma.ttf", "seguisym.ttf"] {
            if let Ok(bytes) = std::fs::read(dir.join(file)) {
                let name = format!("system-{file}");
                fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
                system.push(name);
            }
        }
    }
    // egui's bundled fonts stay as fallbacks for emoji and rare scripts.
    let mut fallbacks: Vec<String> = system;
    fallbacks.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    // The bundled font files are trimmed so they never overlap:
    // Inter has no private-use glyphs and Phosphor has only private-use ones.
    let family = |main: &str| {
        let mut v = vec![main.to_string(), "phosphor".to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(medium(), family("inter-medium"));
    fonts.families.insert(semibold(), family("inter-semibold"));
    fonts.families.insert(bold(), family("inter-bold"));
    fonts.families.entry(FontFamily::Monospace).or_default().push("phosphor".into());
    ctx.set_fonts(fonts);
}

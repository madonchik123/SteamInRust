//! The SteamInRust logo (a piston between two hex nuts on an octagon), from
//! the SVGs in `crates/ui/assets/logos`. Where each version is used:
//!
//! - [`Logo::Purple`]: the app icon (window, taskbar, installer, About);
//! - [`Logo::CutoutWhite`] / [`Logo::CutoutBlack`]: the one-color mark in the
//!   app's own bars, so it sits with the theme's colors (see
//!   [`crate::widgets::logo`]);
//! - [`Logo::OutlineWhite`]: Big Picture's opening;
//! - any of them: the tray icon, picked in Settings → Appearance
//!   ([`Logo::Classic`] unless changed).

use std::fmt;

/// One version of the logo. Saved by [`Logo::key`]; an unknown saved name
/// loads as the default (the tray's, [`Logo::Classic`]) instead of failing
/// the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Logo {
    /// Solid violet octagon with a white piston: the app icon.
    Purple,
    /// Violet fading to black from the top: the tray icon by default.
    #[default]
    Classic,
    Black,
    /// Violet outline.
    Outline,
    OutlineWhite,
    OutlineBlack,
    /// Violet octagon with the piston cut out (see-through).
    Cutout,
    CutoutWhite,
    CutoutBlack,
}

impl Logo {
    pub const ALL: [Logo; 9] = [
        Logo::Purple,
        Logo::Classic,
        Logo::Black,
        Logo::Outline,
        Logo::OutlineWhite,
        Logo::OutlineBlack,
        Logo::Cutout,
        Logo::CutoutWhite,
        Logo::CutoutBlack,
    ];

    /// The name it's saved under in the settings (never change these).
    pub fn key(self) -> &'static str {
        match self {
            Logo::Purple => "purple",
            Logo::Classic => "classic",
            Logo::Black => "black",
            Logo::Outline => "outline",
            Logo::OutlineWhite => "outline-white",
            Logo::OutlineBlack => "outline-black",
            Logo::Cutout => "cutout",
            Logo::CutoutWhite => "cutout-white",
            Logo::CutoutBlack => "cutout-black",
        }
    }

    pub fn from_key(key: &str) -> Option<Logo> {
        Logo::ALL.into_iter().find(|l| l.key() == key)
    }

    /// English name (callers translate it with `i18n::tr_dyn`).
    pub fn name(self) -> &'static str {
        match self {
            Logo::Purple => "Purple",
            Logo::Classic => "Classic",
            Logo::Black => "Black",
            Logo::Outline => "Outline",
            Logo::OutlineWhite => "White outline",
            Logo::OutlineBlack => "Black outline",
            Logo::Cutout => "Cut-out",
            Logo::CutoutWhite => "White cut-out",
            Logo::CutoutBlack => "Black cut-out",
        }
    }

    pub fn svg(self) -> &'static [u8] {
        match self {
            Logo::Purple => include_bytes!("../assets/logos/purple.svg"),
            Logo::Classic => include_bytes!("../assets/logos/classic.svg"),
            Logo::Black => include_bytes!("../assets/logos/black.svg"),
            Logo::Outline => include_bytes!("../assets/logos/outline.svg"),
            Logo::OutlineWhite => include_bytes!("../assets/logos/outline-white.svg"),
            Logo::OutlineBlack => include_bytes!("../assets/logos/outline-black.svg"),
            Logo::Cutout => include_bytes!("../assets/logos/cutout.svg"),
            Logo::CutoutWhite => include_bytes!("../assets/logos/cutout-white.svg"),
            Logo::CutoutBlack => include_bytes!("../assets/logos/cutout-black.svg"),
        }
    }

    /// The picture egui draws for it (cached under a `.svg` URI, so it's
    /// rendered sharp at every size it's shown at).
    pub fn image(self) -> egui::Image<'static> {
        egui::Image::from_bytes(format!("bytes://steaminrust-logo-{}.svg", self.key()), self.svg())
    }

    /// Straight RGBA pixels of it at `size` × `size`.
    pub fn rgba(self, size: u32) -> Vec<u8> {
        render(self.svg(), size)
    }
}

impl fmt::Display for Logo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl serde::Serialize for Logo {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.key())
    }
}

impl<'de> serde::Deserialize<'de> for Logo {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let key = String::deserialize(d)?;
        Ok(Logo::from_key(&key).unwrap_or_default())
    }
}

/// RGBA pixels of the app icon at `size` × `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    Logo::Purple.rgba(size)
}

/// RGBA pixels of the app icon at `size` × `size`, for places without egui's
/// image loaders (the installer).
pub fn logo_rgba(size: u32) -> Vec<u8> {
    Logo::Purple.rgba(size)
}

/// Straight (not premultiplied) RGBA of `svg` at `size` × `size`, centered,
/// as window and tray icons expect.
fn render(svg: &[u8], size: u32) -> Vec<u8> {
    use resvg::{tiny_skia, usvg};
    let blank = || vec![0; (size * size * 4) as usize];
    let Ok(tree) = usvg::Tree::from_data(svg, &usvg::Options::default()) else { return blank() };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size, size) else { return blank() };
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = size as f32 / w.max(h);
    let (dx, dy) = ((size as f32 - w * scale) / 2.0, (size as f32 - h * scale) / 2.0);
    resvg::render(&tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Logo;

    fn alpha(px: &[u8], size: u32, x: u32, y: u32) -> u8 {
        px[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn every_logo_renders() {
        for logo in Logo::ALL {
            for size in [16, 32, 256] {
                let px = logo.rgba(size);
                assert_eq!(px.len(), (size * size * 4) as usize, "{logo:?}");
                // Corners are see-through (the mark is an octagon)...
                assert_eq!(alpha(&px, size, 0, 0), 0, "{logo:?}");
                // ...and something is drawn.
                let drawn = px.chunks(4).filter(|p| p[3] > 200).count();
                assert!(drawn > (size * size / 10) as usize, "{logo:?} at {size}: {drawn}");
            }
        }
    }

    #[test]
    fn app_icon_is_solid() {
        // A filled octagon: opaque a quarter of the way in.
        let size = 64;
        let px = super::icon_rgba(size);
        assert_eq!(alpha(&px, size, size / 4, size / 2), 255);
    }

    #[test]
    fn saved_names_are_stable() {
        // Saved in the app's settings: renaming one would reset users' choice.
        let names: Vec<String> = Logo::ALL.iter().map(|l| serde_json::to_string(l).unwrap()).collect();
        assert_eq!(names[0], "\"purple\"");
        assert_eq!(names[5], "\"outline-black\"");
        assert_eq!(names[8], "\"cutout-black\"");
        for logo in Logo::ALL {
            assert_eq!(serde_json::from_str::<Logo>(&serde_json::to_string(&logo).unwrap()).unwrap(), logo);
        }
        // A name from a newer version doesn't break loading the settings.
        assert_eq!(serde_json::from_str::<Logo>("\"rainbow\"").unwrap(), Logo::Classic);
    }
}

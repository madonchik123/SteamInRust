//! The logo and app icon, from two SVGs in `crates/ui/assets`: `logo.svg` (an
//! S drawn as two stacked hex nuts, blue to violet) and `icon.svg` (the same
//! mark on a dark rounded tile).

/// The mark on transparent, drawn by [`crate::widgets::logo`].
pub const LOGO_SVG: &[u8] = include_bytes!("../assets/logo.svg");

/// Image URI egui caches the logo under (`.svg`, so it's rendered sharp at
/// every size it's shown at).
pub const LOGO_URI: &str = "bytes://steaminrust-logo.svg";

/// The app icon: the mark on a dark rounded tile.
pub const ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

/// RGBA pixels of the app icon at `size` × `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    render(ICON_SVG, size)
}

/// RGBA pixels of the logo (no tile) at `size` × `size`, for places without
/// egui's image loaders (the installer).
pub fn logo_rgba(size: u32) -> Vec<u8> {
    render(LOGO_SVG, size)
}

/// Straight (not premultiplied) RGBA of `svg` at `size` × `size`, as window
/// and tray icons expect.
fn render(svg: &[u8], size: u32) -> Vec<u8> {
    use resvg::{tiny_skia, usvg};
    let blank = || vec![0; (size * size * 4) as usize];
    let Ok(tree) = usvg::Tree::from_data(svg, &usvg::Options::default()) else { return blank() };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size, size) else { return blank() };
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
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
    #[test]
    fn icon_renders() {
        for size in [16, 32, 256] {
            let px = super::icon_rgba(size);
            assert_eq!(px.len(), (size * size * 4) as usize);
            // Corners are see-through, the middle of the tile isn't.
            assert_eq!(px[3], 0);
            let mid = ((size / 2 * size + size / 4) * 4 + 3) as usize;
            assert_eq!(px[mid], 255);
        }
    }
}

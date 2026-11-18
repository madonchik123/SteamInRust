//! Writes the app icon as PNG files: `cargo run -p ui --example icon_png -- <dir> [logo]`,
//! where `logo` is one of the saved names (`purple`, `outline-white`, …; default `purple`, the app icon).
fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let logo: ui::brand::Logo = args.next().map(|n| serde_json::from_str(&format!("\"{n}\"")).expect("unknown logo")).unwrap_or(ui::brand::Logo::Purple);
    for size in [256u32, 128, 64, 48, 32, 24, 16] {
        let rgba = logo.rgba(size);
        let img = image::RgbaImage::from_raw(size, size, rgba).expect("size");
        img.save(dir.join(format!("icon-{size}.png"))).expect("save");
    }
}

//! NV12 (what the H.264 decoder outputs) to RGBA for egui textures.

use egui::Color32;

/// Convert an NV12 picture (limited range) to opaque RGBA pixels.
/// `stride` is the byte pitch of both planes, `uv` starts the chroma plane.
/// BT.709 for HD, BT.601 for SD, like players do when the stream says nothing.
pub fn nv12_to_rgba(y: &[u8], uv: &[u8], stride: usize, width: usize, height: usize, out: &mut Vec<Color32>) {
    out.clear();
    out.reserve(width * height);
    // Fixed-point coefficients (<< 13).
    let (rv, gu, gv, bu) = if height >= 600 { (14686, 1747, 4366, 17305) } else { (13074, 3203, 6660, 16531) };
    let ys = 9539; // 255/219 << 13
    let clamp = |v: i32| (v >> 13).clamp(0, 255) as u8;
    for row in 0..height {
        let yrow = &y[row * stride..row * stride + width];
        let crow = &uv[(row / 2) * stride..(row / 2) * stride + (width + 1) / 2 * 2];
        for (pair, c) in yrow.chunks(2).zip(crow.chunks_exact(2)) {
            let u = c[0] as i32 - 128;
            let v = c[1] as i32 - 128;
            let r_off = rv * v;
            let g_off = -gu * u - gv * v;
            let b_off = bu * u;
            for &yy in pair {
                let l = (yy as i32 - 16) * ys;
                out.push(Color32::from_rgb(clamp(l + r_off), clamp(l + g_off), clamp(l + b_off)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grey_and_white() {
        let y = [16u8, 235, 126, 126];
        let uv = [128u8, 128];
        let mut out = Vec::new();
        super::nv12_to_rgba(&y, &uv, 2, 2, 2, &mut out);
        assert_eq!(out[0], egui::Color32::BLACK);
        assert_eq!(out[1], egui::Color32::WHITE);
        assert!((out[2].r() as i32 - 128).abs() <= 1);
    }
}

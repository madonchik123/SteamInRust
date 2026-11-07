//! The app icon, drawn in code so it stays sharp at any size: a rounded tile
//! with a blue-to-violet gradient and the diamond mark in white.

/// RGBA pixels of the icon at `size` × `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4; // 4×4 supersampling for smooth edges
    let n = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / n;
                    let py = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / n;
                    let c = shade(px, py);
                    // Premultiplied accumulate.
                    acc[0] += c[0] * c[3];
                    acc[1] += c[1] * c[3];
                    acc[2] += c[2] * c[3];
                    acc[3] += c[3];
                }
            }
            let count = (SAMPLES * SAMPLES) as f32;
            let a = acc[3] / count;
            let i = ((y * size + x) * 4) as usize;
            if a > 0.0 {
                out[i] = (acc[0] / acc[3] * 255.0).round() as u8;
                out[i + 1] = (acc[1] / acc[3] * 255.0).round() as u8;
                out[i + 2] = (acc[2] / acc[3] * 255.0).round() as u8;
            }
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Color at (x, y) in 0..1 coordinates; alpha 0 outside the tile.
fn shade(x: f32, y: f32) -> [f32; 4] {
    // Rounded square ("squircle-ish") tile with a small margin.
    let m = 0.04;
    let r = 0.23;
    let (cx, cy) = (x.clamp(m + r, 1.0 - m - r), y.clamp(m + r, 1.0 - m - r));
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    if d > r || x < m || x > 1.0 - m || y < m || y > 1.0 - m {
        return [0.0; 4];
    }
    // Diagonal gradient: bright blue (top-left) to violet (bottom-right).
    let blue = [0.31, 0.55, 1.0];
    let violet = [0.55, 0.36, 0.96];
    let deep = [0.16, 0.20, 0.45];
    let t = ((x + y) / 2.0).clamp(0.0, 1.0);
    let mut c = mix(blue, violet, t);
    // Soft vignette towards the bottom for depth.
    c = mix(c, deep, ((y - 0.55).max(0.0) * 0.9).min(0.45));
    // Gloss on the top edge.
    let gloss = ((0.38 - y).max(0.0) / 0.38) * 0.10;
    c = mix(c, [1.0, 1.0, 1.0], gloss);

    // The mark: a diamond ring with a solid diamond in the middle.
    let (dx, dy) = ((x - 0.5).abs(), (y - 0.5).abs());
    let diamond = dx + dy;
    let white = [1.0, 1.0, 1.0];
    if (0.20..=0.33).contains(&diamond) {
        c = white;
    } else if diamond <= 0.09 {
        c = mix(white, [0.85, 0.88, 1.0], 0.3);
    }
    [c[0], c[1], c[2], 1.0]
}

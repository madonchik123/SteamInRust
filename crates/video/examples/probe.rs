//! Headless check of the player against a real stream:
//! `cargo run -p video --example probe -- <url> [seconds] [seek]`
//! Prints status, clock, frames shown and the buffered range; muted.

use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber_init();
    let mut args = std::env::args().skip(1);
    let url = args.next().filter(|u| !u.is_empty()).unwrap_or_else(|| {
        "https://video.akamai.steamstatic.com/store_trailers/1145360/312686/b0312c49135823cc34946e98148abfb8da71f0fb/1750627549/hls_264_master.m3u8".into()
    });
    let secs: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let seek: Option<f64> = args.next().and_then(|s| s.parse().ok());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ctx = egui::Context::default();
    let src = if url.contains(".m3u8") { video::Source::Hls(url) } else { video::Source::Mp4(url) };
    let mut p = video::Player::new(rt.handle(), &ctx, src, video::Options { muted: true, width: 1280, ..Default::default() });
    let start = Instant::now();
    let mut last_tex = None;
    let mut shown = 0;
    let mut first = None;
    let mut seeked = false;
    let mut last_print = 0.0;
    while start.elapsed().as_secs_f64() < secs {
        p.poll(&ctx);
        if let Some((id, size)) = p.texture() {
            let pos = p.position();
            if last_tex != Some((pos * 1000.0) as i64) {
                last_tex = Some((pos * 1000.0) as i64);
            }
            if first.is_none() {
                first = Some(start.elapsed().as_secs_f64());
                println!("first frame after {:.2}s: {:?} {:?}", first.unwrap(), id, size);
            }
        }
        shown += 1;
        let t = start.elapsed().as_secs_f64();
        if t - last_print >= 1.0 {
            last_print = t;
            println!(
                "t={t:5.2} audio={} status={:?} pos={:6.2} dur={:6.2} buffered={:6.2} q={:?} err={:?}",
                p.has_audio(),
                p.status(),
                p.position(),
                p.duration(),
                p.buffered(),
                p.quality(),
                p.error()
            );
        }
        if let Some(s) = seek {
            if !seeked && t > secs / 2.0 {
                seeked = true;
                println!("-- seek to {s}");
                p.seek(s);
            }
        }
        if p.take_ended() {
            println!("ended at t={t:.2}");
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("polls={shown}");
}

fn tracing_subscriber_init() {}

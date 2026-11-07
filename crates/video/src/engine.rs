//! The decoding threads: video into a short frame queue, audio straight to
//! the sound card (which also keeps the clock).

use crate::mp4::{self, Codec, Sample, Track};
use crate::{Audio, Frame, MAX_FRAMES, Plan, Shared, UNSEEN, mf, wasapi, yuv};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

/// The samples to decode from one downloaded segment (or the whole file of a
/// plain MP4, from the key frame before `target`).
fn samples(plan: &Plan, track: &Track, data: &[u8], target: f64) -> Vec<Sample> {
    if plan.single {
        let start = track.samples.iter().rposition(|s| s.key && track.pts(s) <= target + 0.001).unwrap_or(0);
        track.samples[start..].to_vec()
    } else {
        mp4::parse_fragment(data, track)
    }
}

fn track_of(data: &[u8], video: bool) -> Option<Track> {
    let m = mp4::parse_movie(data).ok()?;
    if video { m.video().cloned() } else { m.audio().cloned() }
}

pub fn video_thread(sh: Arc<Shared>) {
    if let Err(e) = mf::init_thread() {
        return sh.fail(format!("Media Foundation: {e}"));
    }
    let mut dec: Option<(mf::H264, String)> = None;
    let mut track: Option<(String, Track)> = None;
    let mut epoch = u64::MAX;
    let mut target = 0.0;
    let mut seg = 0usize;
    let mut plan: Option<Arc<Plan>> = None;
    let (mut au, mut pics, mut px) = (Vec::new(), Vec::new(), Vec::new());

    while !sh.stopped() {
        {
            let st = sh.lock();
            if st.epoch != epoch || plan.is_none() {
                epoch = st.epoch;
                target = st.target;
                plan = st.video.clone();
                seg = plan.as_ref().map_or(0, |p| p.index_at(target));
                if let Some((d, _)) = dec.as_mut() {
                    d.flush();
                }
            }
            if plan.is_none() {
                drop(sh.wait(st, 100));
                continue;
            }
        }
        let p = plan.clone().unwrap();
        let n = if p.single { 1 } else { p.segs.len() };

        // The track header (init segment, or the whole file).
        let init_url = p.init_url().to_string();
        let Some(init) = sh.wait_data(&init_url, Some(epoch)) else { continue };
        if track.as_ref().is_none_or(|(u, _)| *u != init_url) {
            match track_of(&init, true) {
                Some(t) => {
                    if p.single && t.duration > 0 {
                        sh.lock().duration = t.duration as f64 / t.timescale.max(1) as f64;
                    }
                    track = Some((init_url.clone(), t));
                }
                None => return sh.fail("no H.264 video in this stream"),
            }
        }
        let t = &track.as_ref().unwrap().1;
        let Codec::H264 { nal_len, sps, pps } = &t.codec else { return };
        if dec.as_ref().is_none_or(|(_, u)| *u != init_url) {
            drop(dec.take());
            match mf::H264::new(t.width, t.height) {
                Ok(d) => dec = Some((d, init_url.clone())),
                Err(e) => return sh.fail(format!("H.264 decoder: {e}")),
            }
        }
        let d = &mut dec.as_mut().unwrap().0;

        if seg >= n {
            // End of stream: flush out the last pictures, then wait for a seek.
            tracing::debug!("video: end of video stream (segment {seg}/{n}, epoch {epoch})");
            pics.clear();
            let _ = d.drain(&mut pics);
            for pic in pics.drain(..) {
                emit(&sh, epoch, target, t, pic, &mut px);
            }
            let mut st = sh.lock();
            if st.epoch == epoch {
                st.video_eos = true;
                sh.notify();
                sh.repaint();
            }
            while st.epoch == epoch && !sh.stopped() {
                st = sh.wait(st, 200);
            }
            continue;
        }

        let data = if p.single {
            init.clone()
        } else {
            match sh.wait_data(&p.segs[seg].url, Some(epoch)) {
                Some(d) => d,
                None => continue,
            }
        };
        let fps_dur = 1.0 / 30.0;
        for s in samples(&p, t, &data, target) {
            if sh.epoch() != epoch || sh.stopped() {
                break;
            }
            let Some(bytes) = data.get(s.offset..s.offset + s.size) else { continue };
            mp4::annex_b(bytes, *nal_len, s.key.then_some((sps.as_slice(), pps.as_slice())), &mut au);
            pics.clear();
            let dur = if s.duration > 0 { t.seconds(s.duration as i64) } else { fps_dur };
            if let Err(e) = d.decode(&au, t.pts(&s), dur, &mut pics) {
                tracing::debug!("video: decode: {e}");
            }
            for pic in pics.drain(..) {
                emit(&sh, epoch, target, t, pic, &mut px);
            }
        }
        if sh.epoch() == epoch {
            seg += 1;
        }
    }
}

/// Convert a decoded picture and queue it (waiting for room).
fn emit(sh: &Shared, epoch: u64, target: f64, t: &Track, pic: mf::Picture, px: &mut Vec<egui::Color32>) {
    // Pictures between the key frame and a seek target are only decoded.
    if pic.pts < target - 0.02 {
        return;
    }
    let (w, h) = (t.width as usize, t.height as usize);
    let (w, h) = (if w == 0 { pic.stride } else { w.min(pic.stride) }, if h == 0 { pic.rows } else { h.min(pic.rows) });
    let luma = pic.stride * pic.rows;
    if w == 0 || h == 0 || pic.data.len() < luma + pic.stride * h.div_ceil(2) {
        return;
    }
    yuv::nv12_to_rgba(&pic.data[..luma], &pic.data[luma..], pic.stride, w, h, px);
    let image = egui::ColorImage::new([w, h], std::mem::take(px));
    let mut st = sh.lock();
    while st.epoch == epoch && st.frames.len() >= MAX_FRAMES && !sh.stopped() {
        st = sh.wait(st, 100);
    }
    if st.epoch != epoch {
        return;
    }
    let first = st.frames.is_empty();
    st.frames.push_back(Frame { pts: pic.pts, image });
    st.has_frame = true;
    drop(st);
    sh.notify();
    if first {
        sh.repaint();
    }
}

fn audio_off(sh: &Shared, why: &str) {
    tracing::info!("video: no audio ({why})");
    sh.lock().audio = Audio::Off;
    sh.notify();
    sh.repaint();
}

pub fn audio_thread(sh: Arc<Shared>) {
    if mf::init_thread().is_err() {
        return audio_off(&sh, "Media Foundation");
    }
    // Wait for the playlists.
    let plan = {
        let mut st = sh.lock();
        while !st.ready && !sh.stopped() && st.error.is_none() {
            st = sh.wait(st, 100);
        }
        st.audio_plan.clone()
    };
    if sh.stopped() {
        return;
    }
    let Some(plan) = plan else { return audio_off(&sh, "no audio stream") };
    let Some(init) = sh.wait_data(plan.init_url(), None) else { return };
    let Some(track) = track_of(&init, false) else { return audio_off(&sh, "no AAC track") };
    let Codec::Aac { config, channels, rate } = &track.codec else { return };
    let mut aac = match mf::Aac::new(config, *rate, *channels) {
        Ok(a) => a,
        Err(e) => return audio_off(&sh, &format!("AAC decoder: {e}")),
    };
    let ch_in = aac.channels.max(1) as usize;
    let ch = ch_in.min(2);
    let rate = aac.rate.max(1);
    let mut out = match wasapi::Output::new(rate, ch as u16) {
        Ok(o) => o,
        Err(e) => return audio_off(&sh, &format!("audio device: {e}")),
    };
    sh.lock().audio = Audio::On;
    sh.notify();

    let fr = rate as f64;
    let mut epoch = u64::MAX;
    let mut target = 0.0;
    let mut seg = 0usize;
    let mut pcm: VecDeque<f32> = VecDeque::new();
    // Time of the next sample in `pcm`.
    let mut pcm_pts = 0.0;
    // Time of the first sample written to the device since its last reset.
    let mut first: Option<f64> = None;
    let mut running = false;
    let mut eos = false;
    let mut vol_set = -1.0f32;
    let mut chunks = Vec::new();
    let mut scratch = Vec::new();

    while !sh.stopped() {
        let now = Instant::now();
        let (g, t, want, vol, video_ready) = {
            let mut st = sh.lock();
            if !st.paused && !st.auto_paused && now.duration_since(st.last_poll) > UNSEEN {
                st.auto_paused = true;
            }
            let vol = if st.muted { 0.0 } else { st.volume * st.volume };
            (st.epoch, st.target, !st.paused && !st.auto_paused, vol, st.has_frame || st.video_eos)
        };
        if g != epoch {
            epoch = g;
            target = t;
            let _ = out.reset();
            running = false;
            aac.flush();
            pcm.clear();
            first = None;
            eos = false;
            seg = plan.index_at(t);
        }
        if vol != vol_set && out.set_volume(vol).is_ok() {
            vol_set = vol;
        }

        // Decode ahead, up to two seconds.
        if !eos && pcm.len() < 2 * rate as usize * ch {
            let n = if plan.single { 1 } else { plan.segs.len() };
            chunks.clear();
            if seg >= n {
                let _ = aac.drain(&mut chunks);
                eos = true;
            } else {
                let url = if plan.single { plan.init_url() } else { &plan.segs[seg].url };
                let data = sh.lock().cache.get(url).cloned();
                if let Some(data) = data {
                    for s in samples(&plan, &track, &data, target) {
                        let Some(bytes) = data.get(s.offset..s.offset + s.size) else { continue };
                        if let Err(e) = aac.decode(bytes, track.pts(&s), track.seconds(s.duration as i64), &mut chunks) {
                            tracing::debug!("video: aac: {e}");
                        }
                    }
                    seg += 1;
                }
            }
            for (pts, buf) in chunks.drain(..) {
                let frames = buf.len() / ch_in;
                let skip = (((target - pts) * fr).ceil().max(0.0) as usize).min(frames);
                if skip == frames {
                    continue;
                }
                if pcm.is_empty() {
                    pcm_pts = pts + skip as f64 / fr;
                }
                for f in skip..frames {
                    let s = &buf[f * ch_in..f * ch_in + ch_in];
                    pcm.extend(&s[..ch]);
                }
            }
        }

        // Feed the device.
        let result = (|| -> windows::core::Result<()> {
            let free = out.free()? as usize;
            let n = free.min(pcm.len() / ch);
            if n > 0 {
                if first.is_none() {
                    first = Some(pcm_pts);
                }
                scratch.clear();
                scratch.extend(pcm.drain(..n * ch));
                out.write(&scratch)?;
                pcm_pts += n as f64 / fr;
            }
            let queued = out.queued()? as usize;
            let ready = eos || pcm.len() / ch + queued >= rate as usize / 4;
            if want && !running && ready && video_ready {
                out.start()?;
                running = true;
            } else if running && !want {
                out.stop()?;
                running = false;
            } else if running && pcm.is_empty() && !eos && queued < rate as usize / 50 {
                // Out of data: hold the clock until more arrives.
                out.stop()?;
                running = false;
            }
            let played = match first {
                Some(f) => Some(f + out.played()?),
                None => None,
            };
            let mut st = sh.lock();
            if st.epoch == epoch {
                if let Some(p) = played {
                    st.clock = p;
                }
                st.clock_at = Instant::now();
                st.running = running;
                st.audio_eos = eos && pcm.is_empty() && queued == 0;
            }
            Ok(())
        })();
        if let Err(e) = result {
            // Usually the device went away (unplugged, default changed): reopen.
            tracing::info!("video: audio device: {e}");
            running = false;
            first = None;
            match wasapi::Output::new(rate, ch as u16) {
                Ok(o) => {
                    out = o;
                    vol_set = -1.0;
                }
                Err(_) => {
                    let mut st = sh.lock();
                    st.running = false;
                    drop(st);
                    return audio_off(&sh, "device lost");
                }
            }
            continue;
        }
        if running {
            out.wait(20);
        } else {
            // Paused: wake rarely (any state change notifies anyway).
            let st = sh.lock();
            drop(sh.wait(st, if want { 20 } else { 100 }));
        }
    }
}

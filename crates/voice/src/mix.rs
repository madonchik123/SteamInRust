//! Evens out each person's packets, hides lost ones, and mixes everyone.

use crate::codec::{Decoder, FRAME, MAX_DECODED};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Packets to hold before starting to play someone (40 ms of cushion).
const PREBUFFER: usize = 2;
/// More than this waiting means we've fallen behind: skip ahead.
const MAX_QUEUE: usize = 15;
const KEEP_AFTER_SKIP: usize = 4;
/// How many lost frames in a row to make up before going quiet.
const MAX_CONCEAL: u32 = 5;
/// Louder than this counts as talking.
const TALKING_LEVEL: f32 = 0.012;
/// Frames someone still counts as talking after they go quiet (300 ms).
const TALKING_HOLD: u32 = 15;

struct Speaker {
    decoder: Decoder,
    queue: BTreeMap<u64, Vec<u8>>,
    /// The next packet to play; `None` while filling the cushion.
    next: Option<u64>,
    decoded: VecDeque<f32>,
    concealed: u32,
    gain: f32,
    talking: u32,
}

impl Speaker {
    fn new(gain: f32) -> Result<Self, String> {
        Ok(Self { decoder: Decoder::new()?, queue: BTreeMap::new(), next: None, decoded: VecDeque::new(), concealed: 0, gain, talking: 0 })
    }

    fn push(&mut self, seq: u64, payload: &[u8]) {
        if self.next.is_some_and(|n| seq < n) {
            return; // too late
        }
        self.queue.insert(seq, payload.to_vec());
        if self.queue.len() > MAX_QUEUE {
            while self.queue.len() > KEEP_AFTER_SKIP {
                self.queue.pop_first();
            }
            if self.next.is_some() {
                self.next = self.queue.keys().next().copied();
            }
        }
    }

    /// Decode the next packet's worth of sound; false when there's none.
    fn decode_next(&mut self) -> bool {
        let mut pcm = [0f32; MAX_DECODED];
        let next = match self.next {
            Some(n) => n,
            None => {
                if self.queue.len() < PREBUFFER {
                    return false;
                }
                let first = *self.queue.keys().next().expect("the queue isn't empty");
                self.next = Some(first);
                first
            }
        };
        let n = if let Some(packet) = self.queue.remove(&next) {
            self.concealed = 0;
            self.next = Some(next + 1);
            self.decoder.decode(&packet, &mut pcm)
        } else if self.queue.is_empty() {
            // Nothing is coming: they stopped talking, or packets are late.
            if self.concealed >= MAX_CONCEAL {
                self.next = None;
                self.concealed = 0;
                return false;
            }
            self.concealed += 1;
            self.next = Some(next + 1);
            self.decoder.conceal(&mut pcm)
        } else {
            // A packet is missing but later ones are here.
            self.next = Some(next + 1);
            match self.queue.get(&(next + 1)) {
                Some(after) => self.decoder.recover(after, &mut pcm),
                None => self.decoder.conceal(&mut pcm),
            }
        };
        match n {
            Ok(n) => {
                self.decoded.extend(&pcm[..n]);
                true
            }
            Err(e) => {
                tracing::debug!("voice: {e}");
                false
            }
        }
    }

    /// Add this person's next 20 ms to `out`.
    fn mix_into(&mut self, out: &mut [f32]) {
        while self.decoded.len() < FRAME {
            if !self.decode_next() {
                break;
            }
        }
        if self.decoded.len() < FRAME {
            // Not enough for a whole frame: wait, rather than play a click.
            if self.next.is_none() {
                self.decoded.clear();
            }
            self.talking = self.talking.saturating_sub(1);
            return;
        }
        let mut power = 0.0;
        for slot in out.iter_mut().take(FRAME) {
            let s = self.decoded.pop_front().unwrap_or(0.0);
            power += s * s;
            *slot += s * self.gain;
        }
        if (power / FRAME as f32).sqrt() > TALKING_LEVEL {
            self.talking = TALKING_HOLD;
        } else {
            self.talking = self.talking.saturating_sub(1);
        }
    }
}

/// Everyone we're hearing, by their stream number (SSRC).
#[derive(Default)]
pub struct Mixer {
    speakers: HashMap<u32, Speaker>,
    gains: HashMap<u32, f32>,
    master: f32,
    deaf: bool,
}

impl Mixer {
    pub fn new(master: f32) -> Self {
        Self { master, ..Default::default() }
    }

    pub fn push(&mut self, ssrc: u32, seq: u64, payload: &[u8]) {
        if !self.speakers.contains_key(&ssrc) {
            let gain = self.gains.get(&ssrc).copied().unwrap_or(1.0);
            match Speaker::new(gain) {
                Ok(s) => {
                    self.speakers.insert(ssrc, s);
                }
                Err(e) => {
                    tracing::warn!("voice: no decoder: {e}");
                    return;
                }
            }
        }
        if let Some(s) = self.speakers.get_mut(&ssrc) {
            s.push(seq, payload);
        }
    }

    /// One person's volume, 0 to 4 like Steam's.
    pub fn set_gain(&mut self, ssrc: u32, gain: f32) {
        let gain = gain.clamp(0.0, 4.0);
        self.gains.insert(ssrc, gain);
        if let Some(s) = self.speakers.get_mut(&ssrc) {
            s.gain = gain;
        }
    }

    pub fn set_master(&mut self, gain: f32) {
        self.master = gain.clamp(0.0, 4.0);
    }

    pub fn set_deaf(&mut self, deaf: bool) {
        self.deaf = deaf;
    }

    /// Forget streams that are gone.
    pub fn retain(&mut self, ssrcs: &[u32]) {
        self.speakers.retain(|id, _| ssrcs.contains(id));
    }

    /// The next 20 ms of everyone together.
    pub fn pull(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for s in self.speakers.values_mut() {
            s.mix_into(out);
        }
        if self.deaf {
            out.fill(0.0);
            return;
        }
        for s in out.iter_mut() {
            // Keeps several loud people from clipping harshly.
            *s = soft_clip(*s * self.master);
        }
    }

    /// Who's talking right now, sorted.
    pub fn talking(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.speakers.iter().filter(|(_, s)| s.talking > 0).map(|(id, _)| *id).collect();
        v.sort_unstable();
        v
    }
}

fn soft_clip(s: f32) -> f32 {
    if s.abs() <= 0.8 { s } else { s.signum() * (0.8 + 0.2 * (1.0 - (-(s.abs() - 0.8) / 0.2).exp())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Encoder, pitch, tone};

    fn packets(hz: f32, n: usize) -> Vec<Vec<u8>> {
        let mut e = Encoder::new().unwrap();
        (0..n)
            .map(|f| {
                let mut p = [0u8; 400];
                let len = e.encode(&tone(f, hz), &mut p).unwrap();
                p[..len].to_vec()
            })
            .collect()
    }

    fn pull_all(m: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..frames {
            let mut f = [0f32; FRAME];
            m.pull(&mut f);
            all.extend_from_slice(&f);
        }
        all
    }

    /// Packets arriving in step with playback, like a real call.
    fn play(m: &mut Mixer, people: &[(u32, &[Vec<u8>])]) -> Vec<f32> {
        let n = people.iter().map(|(_, p)| p.len()).max().unwrap_or(0);
        let mut all = Vec::new();
        for i in 0..n {
            for (id, packets) in people {
                if let Some(p) = packets.get(i) {
                    m.push(*id, i as u64, p);
                }
            }
            all.extend(pull_all(m, 1));
        }
        all
    }

    fn loudness(pcm: &[f32]) -> f32 {
        (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt()
    }

    #[test]
    fn waits_for_a_cushion_then_plays() {
        let mut m = Mixer::new(1.0);
        let ps = packets(440.0, 40);
        m.push(1, 100, &ps[0]);
        assert_eq!(loudness(&pull_all(&mut m, 1)), 0.0, "one packet isn't enough to start");
        for (i, p) in ps.iter().enumerate().skip(1) {
            m.push(1, 100 + i as u64, p);
            let mut f = [0f32; FRAME];
            m.pull(&mut f);
        }
        assert_eq!(m.talking(), [1]);
    }

    #[test]
    fn out_of_order_packets_play_in_order() {
        let ps = packets(440.0, 60);
        let mut m = Mixer::new(1.0);
        let mut heard = Vec::new();
        for chunk in (0..60u64).collect::<Vec<_>>().chunks(2) {
            for &i in chunk.iter().rev() {
                m.push(1, i, &ps[i as usize]);
            }
            heard.extend(pull_all(&mut m, 2));
        }
        let hz = pitch(&heard[FRAME * 15..FRAME * 50]);
        assert!((hz - 440.0).abs() < 12.0, "heard {hz} Hz");
    }

    #[test]
    fn a_lost_packet_doesnt_stop_the_sound() {
        let ps = packets(440.0, 60);
        let mut m = Mixer::new(1.0);
        let mut heard = Vec::new();
        for i in 0..60u64 {
            if i != 30 {
                m.push(1, i, &ps[i as usize]);
            }
            heard.extend(pull_all(&mut m, 1));
        }
        let around_gap = &heard[FRAME * 29..FRAME * 35];
        assert!(around_gap.chunks(FRAME).all(|f| loudness(f) > 0.05), "a silent hole in the sound");
    }

    #[test]
    fn volume_per_person_and_deafen() {
        let ps = packets(440.0, 50);
        let mut m = Mixer::new(1.0);
        m.set_gain(1, 0.0);
        let heard = play(&mut m, &[(1, &ps)]);
        assert_eq!(loudness(&heard), 0.0, "person 1 is at zero");
        assert_eq!(m.talking(), [1], "still shown as talking when turned down");

        let mut m = Mixer::new(1.0);
        let heard = play(&mut m, &[(2, &ps[..40])]);
        assert!(loudness(&heard[FRAME * 8..]) > 0.1);
        m.set_deaf(true);
        for (i, p) in ps[40..].iter().enumerate() {
            m.push(2, 40 + i as u64, p);
        }
        assert_eq!(loudness(&pull_all(&mut m, 5)), 0.0, "deafened");
        assert_eq!(m.talking(), [2], "you can still see who's talking");
    }

    #[test]
    fn two_people_are_mixed() {
        let (a, b) = (packets(300.0, 50), packets(1000.0, 50));
        let mut m = Mixer::new(1.0);
        let all = play(&mut m, &[(1, &a), (2, &b)]);
        assert_eq!(m.talking(), [1, 2]);
        let hz = pitch(&all[FRAME * 10..]);
        assert!(hz > 320.0, "both are in the mix ({hz} Hz)");
    }

    #[test]
    fn falling_far_behind_skips_ahead() {
        let ps = packets(440.0, 60);
        let mut m = Mixer::new(1.0);
        for (i, p) in ps.iter().enumerate() {
            m.push(1, i as u64, p);
        }
        let s = m.speakers.get(&1).unwrap();
        assert!(s.queue.len() <= MAX_QUEUE, "{} waiting", s.queue.len());
    }

    #[test]
    fn a_person_who_stops_goes_quiet() {
        let ps = packets(440.0, 12);
        let mut m = Mixer::new(1.0);
        for (i, p) in ps.iter().enumerate() {
            m.push(1, i as u64, p);
        }
        pull_all(&mut m, 12 + MAX_CONCEAL as usize + TALKING_HOLD as usize + 2);
        assert!(m.talking().is_empty());
        assert_eq!(loudness(&pull_all(&mut m, 2)), 0.0);
    }

    #[test]
    fn loud_mixes_stay_in_range() {
        for s in [-4.0f32, -1.0, 0.0, 0.5, 1.0, 4.0] {
            assert!(soft_clip(s).abs() <= 1.0);
        }
        assert_eq!(soft_clip(0.5), 0.5);
    }
}

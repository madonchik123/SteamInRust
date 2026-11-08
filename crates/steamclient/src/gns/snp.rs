//! SNP, the message layer inside SteamNetworkingSockets packets
//! (SNP_WIRE_FORMAT.md and steamnetworkingsockets_snp.cpp in Valve's
//! GameNetworkingSockets): unreliable message segments, a reliable byte
//! stream carrying framed messages, acks with RLE nack blocks, and
//! "stop waiting". One lane (lane 0); frames for other lanes are read and
//! delivered like lane 0.
//!
//! Packet numbers: the handshake counts as packet 1 on both sides, so data
//! starts at 2. On the wire only the low 16 bits are sent.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Most plaintext we put in one packet (leaves room for transport headers,
/// the GCM tag and relay overhead under a 1300-byte MTU).
pub const MAX_PLAINTEXT: usize = 1100;
/// Biggest message we take (Valve's k_cbMaxSteamNetworkingSocketsMessageSizeSend).
pub const MAX_MESSAGE: usize = 512 * 1024;
const MAX_RECV_STREAM: usize = 4 * 1024 * 1024;

/// A complete message from the peer.
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub data: Vec<u8>,
    pub num: i64,
    pub reliable: bool,
    pub lane: u32,
}

struct Seg {
    end: u64,
    data: Vec<u8>,
    acked: bool,
    /// Needs to go out again (lost, or never sent).
    retry: bool,
}

struct InFlight {
    sent: Instant,
    /// Reliable segments (by stream begin) in this packet.
    segs: Vec<u64>,
    nacked: bool,
}

#[derive(Default)]
struct Partial {
    parts: BTreeMap<u32, Vec<u8>>,
    total: Option<u32>,
    since: Option<Instant>,
}

pub struct Snp {
    // ---- sending
    next_pkt: u64,
    next_msg: i64,
    last_rel_msg: i64,
    /// Next stream position for new reliable bytes (position 0 is reserved).
    stream_end: u64,
    /// Reliable bytes framed but not yet in a segment.
    unsent: VecDeque<u8>,
    segs: BTreeMap<u64, Seg>,
    unreliable: VecDeque<(i64, Vec<u8>, u32)>,
    in_flight: BTreeMap<u64, InFlight>,
    /// Packets below this are settled (acked or given up on).
    min_waiting: u64,
    stop_waiting_sent: u64,
    pub rtt: Option<Duration>,

    // ---- receiving
    max_recv: u64,
    max_recv_at: Option<Instant>,
    /// Packets received at or above `ack_floor`.
    received: BTreeSet<u64>,
    ack_floor: u64,
    /// We owe the peer an ack, by this time.
    ack_due: Option<Instant>,
    recv_pos: u64,
    recv_buf: Vec<u8>,
    /// Holes in recv_buf: begin -> end.
    recv_gaps: BTreeMap<u64, u64>,
    last_recv_rel_msg: i64,
    highest_msg: i64,
    partials: HashMap<i64, Partial>,
    pub inbox: VecDeque<Msg>,
    pub last_recv: Option<Instant>,
    /// Bytes waiting to be sent (for status calls).
    pub pending_bytes: usize,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(d: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *d.get(*at)?;
        *at += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn take<'a>(d: &'a [u8], at: &mut usize, n: usize) -> Option<&'a [u8]> {
    let s = d.get(*at..*at + n)?;
    *at += n;
    Some(s)
}

fn le(d: &[u8]) -> u64 {
    d.iter().rev().fold(0u64, |a, &b| a << 8 | b as u64)
}

/// The full number nearest `reference` whose low bits are `low` (Valve's NearestWithSameLowerBits).
pub fn nearest(low: u64, bits: u32, reference: u64) -> u64 {
    let span = 1u64 << bits;
    let mask = span - 1;
    let base = reference & !mask;
    let cands = [base.wrapping_sub(span) | low, base | low, (base + span) | low];
    *cands.iter().filter(|&&c| c < u64::MAX / 2).min_by_key(|&&c| c.abs_diff(reference)).unwrap_or(&(base | low))
}

/// "Size of data" bits and byte for a segment.
fn put_size(out: &mut Vec<u8>, lead: &mut u8, len: usize) {
    debug_assert!(len <= 0x4ff);
    *lead |= (len >> 8) as u8;
    out.push(len as u8);
}

impl Default for Snp {
    fn default() -> Self {
        Self::new()
    }
}

impl Snp {
    pub fn new() -> Self {
        Snp {
            next_pkt: 2,
            next_msg: 1,
            last_rel_msg: 0,
            stream_end: 1,
            unsent: VecDeque::new(),
            segs: BTreeMap::new(),
            unreliable: VecDeque::new(),
            in_flight: BTreeMap::new(),
            min_waiting: 2,
            stop_waiting_sent: 0,
            rtt: None,
            max_recv: 1,
            max_recv_at: None,
            received: BTreeSet::new(),
            ack_floor: 2,
            ack_due: None,
            recv_pos: 1,
            recv_buf: Vec::new(),
            recv_gaps: BTreeMap::new(),
            last_recv_rel_msg: 0,
            highest_msg: 0,
            partials: HashMap::new(),
            inbox: VecDeque::new(),
            last_recv: None,
            pending_bytes: 0,
        }
    }

    // ------------------------------------------------------------ sending

    /// Queue a message; returns its message number.
    pub fn queue(&mut self, data: &[u8], reliable: bool) -> i64 {
        let num = self.next_msg;
        self.next_msg += 1;
        self.pending_bytes += data.len();
        if reliable {
            let mut hdr = Vec::with_capacity(12);
            let jump = num != self.last_rel_msg + 1;
            let len = data.len();
            let mut lead = if jump { 0x40 } else { 0 };
            lead |= if len < 32 { len as u8 } else { 0x20 | (len & 0x1f) as u8 };
            hdr.push(lead);
            if jump {
                put_varint(&mut hdr, (num - self.last_rel_msg) as u64);
            }
            if len >= 32 {
                put_varint(&mut hdr, (len >> 5) as u64);
            }
            self.last_rel_msg = num;
            self.unsent.extend(hdr);
            self.unsent.extend(data.iter().copied());
        } else {
            self.unreliable.push_back((num, data.to_vec(), 0));
        }
        num
    }

    /// Something to send right now (data, a retry, or an ack we owe).
    pub fn wants_send(&self, now: Instant) -> bool {
        !self.unsent.is_empty() || !self.unreliable.is_empty() || self.segs.values().any(|s| s.retry && !s.acked) || self.ack_due.is_some_and(|t| t <= now)
    }

    pub fn has_unacked(&self) -> bool {
        !self.segs.is_empty() || !self.unsent.is_empty()
    }

    /// The ack frame for what we've received, if any.
    fn ack_frame(&self, now: Instant, out: &mut Vec<u8>) {
        if self.max_recv < self.ack_floor {
            return;
        }
        let latest = self.max_recv;
        let mut blocks: Vec<(u64, u64)> = Vec::new();
        let mut pos = latest; // next packet to account for (going down)
        let floor = self.ack_floor;
        loop {
            let mut acks = 0u64;
            while pos >= floor && self.received.contains(&pos) {
                acks += 1;
                pos -= 1;
            }
            if pos < floor {
                // Everything left is received: the implicit block covers it.
                if !blocks.is_empty() || acks == 0 {
                    // nothing more to say
                }
                break;
            }
            let mut nacks = 0u64;
            while pos >= floor && !self.received.contains(&pos) {
                nacks += 1;
                pos -= 1;
            }
            blocks.push((acks, nacks));
            if pos < floor || blocks.len() >= 255 {
                break;
            }
        }
        let delay = self.max_recv_at.map(|t| (now.saturating_duration_since(t).as_micros() >> 5).min(0xfffe) as u16).unwrap_or(0xffff);
        let n = blocks.len();
        out.push(0x98 | if n >= 7 { 7 } else { n as u8 });
        out.extend_from_slice(&(latest as u16).to_le_bytes());
        out.extend_from_slice(&delay.to_le_bytes());
        if n >= 7 {
            out.push(n as u8);
        }
        for (a, k) in blocks {
            let mut b = 0u8;
            b |= if a < 8 { (a as u8) << 4 } else { 0x80 | ((a & 7) as u8) << 4 };
            b |= if k < 8 { k as u8 } else { 0x08 | (k & 7) as u8 };
            out.push(b);
            if a >= 8 {
                put_varint(out, a >> 3);
            }
            if k >= 8 {
                put_varint(out, k >> 3);
            }
        }
    }

    /// Build the next packet's plaintext: (packet number, frames). `force`:
    /// send even if empty (keepalive / ping).
    pub fn build(&mut self, now: Instant, force: bool) -> Option<(u64, Vec<u8>)> {
        let has_data = !self.unsent.is_empty() || !self.unreliable.is_empty() || self.segs.values().any(|s| s.retry && !s.acked);
        let owe_ack = self.ack_due.is_some();
        if !has_data && !owe_ack && !force {
            return None;
        }
        let pkt = self.next_pkt;
        self.next_pkt += 1;
        let mut out = Vec::with_capacity(MAX_PLAINTEXT);

        // Stop waiting: packets below min_waiting are settled.
        let stop = self.in_flight.iter().find(|(_, f)| !f.nacked).map(|(n, _)| *n).unwrap_or(pkt).min(pkt - 1);
        if stop > self.stop_waiting_sent && stop > 2 {
            let off = pkt - stop - 1;
            if off < 0x100 {
                out.push(0x80);
                out.push(off as u8);
            } else if off < 0x1_0000 {
                out.push(0x81);
                out.extend_from_slice(&(off as u16).to_le_bytes());
            } else {
                out.push(0x83);
                out.extend_from_slice(&off.to_le_bytes());
            }
            self.stop_waiting_sent = stop;
        }
        self.ack_frame(now, &mut out);
        self.ack_due = None;

        let mut segs_in_pkt = Vec::new();
        let mut cur_unrel: i64 = 0; // mirrors the decoder's message number
        let mut last_rel_end: Option<u64> = None;

        // Retries first, then new reliable bytes.
        let retry: Vec<u64> = self.segs.iter().filter(|(_, s)| s.retry && !s.acked).map(|(b, _)| *b).collect();
        for b in retry {
            let len = self.segs[&b].data.len();
            if out.len() + len + 8 > MAX_PLAINTEXT {
                break;
            }
            let s = self.segs.get_mut(&b).unwrap();
            s.retry = false;
            Self::put_reliable(&mut out, &mut last_rel_end, b, &s.data);
            if cur_unrel > 0 {
                cur_unrel += 1;
            }
            segs_in_pkt.push(b);
        }
        while !self.unsent.is_empty() {
            let room = MAX_PLAINTEXT.saturating_sub(out.len() + 8);
            if room < 32 {
                break;
            }
            let n = room.min(self.unsent.len()).min(1000);
            let data: Vec<u8> = self.unsent.drain(..n).collect();
            let b = self.stream_end;
            self.stream_end += n as u64;
            Self::put_reliable(&mut out, &mut last_rel_end, b, &data);
            if cur_unrel > 0 {
                cur_unrel += 1;
            }
            self.segs.insert(b, Seg { end: b + n as u64, data, acked: false, retry: false });
            segs_in_pkt.push(b);
        }
        // Unreliable messages (split when big).
        while let Some((num, data, offset)) = self.unreliable.pop_front() {
            let room = MAX_PLAINTEXT.saturating_sub(out.len() + 12);
            if room < 16 {
                self.unreliable.push_front((num, data, offset));
                break;
            }
            let rest = data.len() - offset as usize;
            let n = rest.min(room).min(1000);
            let last = n == rest;
            let mut lead = if last { 0x20 } else { 0 };
            let mut body = Vec::new();
            if cur_unrel == 0 {
                body.extend_from_slice(&(num as u16).to_le_bytes());
            } else if num != cur_unrel + 1 {
                lead |= 0x10;
                put_varint(&mut body, (num - cur_unrel) as u64);
            }
            cur_unrel = num;
            if offset > 0 {
                lead |= 0x08;
                put_varint(&mut body, offset as u64);
            }
            put_size(&mut body, &mut lead, n);
            out.push(lead);
            out.extend_from_slice(&body);
            out.extend_from_slice(&data[offset as usize..offset as usize + n]);
            self.pending_bytes = self.pending_bytes.saturating_sub(n);
            if !last {
                self.unreliable.push_front((num, data, offset + n as u32));
            }
        }
        if !segs_in_pkt.is_empty() || has_data || force {
            self.in_flight.insert(pkt, InFlight { sent: now, segs: segs_in_pkt, nacked: false });
        }
        Some((pkt, out))
    }

    fn put_reliable(out: &mut Vec<u8>, last_end: &mut Option<u64>, begin: u64, data: &[u8]) {
        let mut lead = 0x40u8;
        let mut body = Vec::new();
        match *last_end {
            None => {
                lead |= 1 << 3; // 32-bit absolute position
                body.extend_from_slice(&(begin as u32).to_le_bytes());
            }
            Some(prev) if begin == prev => {}
            Some(prev) => {
                // Only forward offsets are encodable; go absolute otherwise.
                let off = begin.wrapping_sub(prev);
                if begin < prev {
                    unreachable!("reliable segments are written in order")
                } else if off < 0x100 {
                    lead |= 1 << 3;
                    body.push(off as u8);
                } else if off < 0x1_0000 {
                    lead |= 2 << 3;
                    body.extend_from_slice(&(off as u16).to_le_bytes());
                } else {
                    lead |= 3 << 3;
                    body.extend_from_slice(&(off as u32).to_le_bytes());
                }
            }
        }
        put_size(&mut body, &mut lead, data.len());
        out.push(lead);
        out.extend_from_slice(&body);
        out.extend_from_slice(data);
        *last_end = Some(begin + data.len() as u64);
    }

    /// Loss detection: packets not acked in time count as lost.
    pub fn tick(&mut self, now: Instant) {
        let rto = self.rtt.map(|r| r * 2 + Duration::from_millis(50)).unwrap_or(Duration::from_millis(300)).clamp(Duration::from_millis(100), Duration::from_secs(2));
        let late: Vec<u64> = self.in_flight.iter().filter(|(_, f)| !f.nacked && now.duration_since(f.sent) > rto).map(|(n, _)| *n).collect();
        for n in late {
            self.nack(n);
        }
        // Forget very old records.
        let old: Vec<u64> = self.in_flight.iter().filter(|(_, f)| f.nacked && now.duration_since(f.sent) > Duration::from_secs(5)).map(|(n, _)| *n).collect();
        for n in old {
            self.in_flight.remove(&n);
        }
        self.partials.retain(|_, p| p.since.is_some_and(|s| now.duration_since(s) < Duration::from_secs(5)));
    }

    fn nack(&mut self, pkt: u64) {
        let Some(f) = self.in_flight.get_mut(&pkt) else { return };
        if f.nacked {
            return;
        }
        f.nacked = true;
        for b in std::mem::take(&mut f.segs) {
            if let Some(s) = self.segs.get_mut(&b)
                && !s.acked
            {
                s.retry = true;
            }
        }
    }

    fn acked(&mut self, pkt: u64, now: Instant, latest: bool, delay_us: Option<u64>) {
        let Some(f) = self.in_flight.remove(&pkt) else { return };
        if latest && let Some(d) = delay_us {
            let rtt = now.duration_since(f.sent).saturating_sub(Duration::from_micros(d));
            self.rtt = Some(match self.rtt {
                Some(r) => (r * 7 + rtt) / 8,
                None => rtt,
            });
        }
        for b in f.segs {
            if let Some(s) = self.segs.get_mut(&b) {
                s.acked = true;
                s.retry = false;
            }
        }
        // Drop acked segments from the front of the stream.
        while let Some((&b, s)) = self.segs.iter().next() {
            if !s.acked {
                break;
            }
            let _ = b;
            let (_, s) = self.segs.pop_first().unwrap();
            self.pending_bytes = self.pending_bytes.saturating_sub(s.data.len());
        }
    }

    // ---------------------------------------------------------- receiving

    /// Expand a 16-bit wire packet number; None for duplicates and old packets.
    pub fn expand(&self, wire: u16) -> Option<u64> {
        let n = nearest(wire as u64, 16, self.max_recv);
        if n < self.ack_floor || n <= 1 || self.received.contains(&n) {
            return None;
        }
        Some(n)
    }

    /// Process a decrypted packet's frames. Err: the peer broke the protocol.
    pub fn receive(&mut self, pkt: u64, d: &[u8], now: Instant) -> Result<(), String> {
        let mut at = 0usize;
        let mut cur_unrel: i64 = 0;
        let mut rel_pos: u64 = 0;
        let mut lane = 0u32;
        let mut had_reliable = false;
        let bad = |w: &str| Err(format!("SNP decode error: {w}"));
        while at < d.len() {
            let lead = d[at];
            at += 1;
            let size = |at: &mut usize| -> Option<usize> {
                let s = (lead & 7) as usize;
                if s <= 4 {
                    let lo = *d.get(*at)? as usize;
                    *at += 1;
                    let n = s << 8 | lo;
                    (*at + n <= d.len()).then_some(n)
                } else if s == 7 {
                    Some(d.len() - *at)
                } else {
                    None
                }
            };
            if lead & 0xc0 == 0 {
                // Unreliable segment
                if cur_unrel == 0 {
                    let (low, bits) = if lead & 0x10 != 0 { (take(d, &mut at, 4).map(le), 32) } else { (take(d, &mut at, 2).map(le), 16) };
                    let Some(low) = low else { return bad("unreliable msgnum") };
                    cur_unrel = nearest(low, bits, self.highest_msg.max(0) as u64) as i64;
                } else if lead & 0x10 != 0 {
                    let Some(o) = get_varint(d, &mut at) else { return bad("unreliable offset") };
                    cur_unrel += o as i64;
                } else {
                    cur_unrel += 1;
                }
                self.highest_msg = self.highest_msg.max(cur_unrel);
                let offset = if lead & 0x08 != 0 {
                    match get_varint(d, &mut at) {
                        Some(o) => o as u32,
                        None => return bad("unreliable data offset"),
                    }
                } else {
                    0
                };
                let Some(n) = size(&mut at) else { return bad("unreliable size") };
                let data = &d[at..at + n];
                at += n;
                self.unreliable_segment(cur_unrel, offset, data, lead & 0x20 != 0, lane, now);
            } else if lead & 0xe0 == 0x40 {
                // Reliable segment
                if rel_pos == 0 {
                    let (bits, n) = match lead >> 3 & 3 {
                        0 => (24, 3),
                        1 => (32, 4),
                        2 => (48, 6),
                        _ => return bad("reliable pos size"),
                    };
                    let Some(low) = take(d, &mut at, n).map(le) else { return bad("reliable pos") };
                    let expect = self.recv_pos + self.recv_buf.len() as u64;
                    rel_pos = nearest(low, bits, expect);
                } else {
                    let off = match lead >> 3 & 3 {
                        0 => Some(0),
                        1 => take(d, &mut at, 1).map(le),
                        2 => take(d, &mut at, 2).map(le),
                        _ => take(d, &mut at, 4).map(le),
                    };
                    let Some(off) = off else { return bad("reliable offset") };
                    rel_pos += off;
                }
                let Some(n) = size(&mut at) else { return bad("reliable size") };
                let data = d[at..at + n].to_vec();
                at += n;
                self.reliable_segment(rel_pos, &data, lane)?;
                rel_pos += n as u64;
                had_reliable = true;
                if cur_unrel > 0 {
                    cur_unrel += 1;
                }
            } else if lead & 0xfc == 0x80 {
                // Stop waiting
                let off = match lead & 3 {
                    0 => take(d, &mut at, 1).map(le),
                    1 => take(d, &mut at, 2).map(le),
                    2 => take(d, &mut at, 3).map(le),
                    _ => take(d, &mut at, 8).map(le),
                };
                let Some(off) = off else { return bad("stop waiting") };
                let min = pkt.checked_sub(off + 1).ok_or("SNP stop_waiting underflow")?;
                let min = min.min(self.max_recv);
                if min > self.ack_floor {
                    self.ack_floor = min;
                    self.received = self.received.split_off(&min);
                }
            } else if lead & 0xf0 == 0x90 {
                // Ack
                let wide = lead & 0x08 == 0;
                let low = if wide { take(d, &mut at, 4).map(le) } else { take(d, &mut at, 2).map(le) };
                let Some(low) = low else { return bad("ack latest") };
                let latest = nearest(low, if wide { 32 } else { 16 }, self.next_pkt);
                let Some(delay) = take(d, &mut at, 2).map(le) else { return bad("ack delay") };
                let mut blocks = (lead & 7) as u64;
                if blocks == 7 {
                    let Some(b) = take(d, &mut at, 1).map(le) else { return bad("ack blocks") };
                    blocks = b;
                }
                if latest >= self.next_pkt {
                    // Acks for packets we never sent: ignore the rest of this packet.
                    return Ok(());
                }
                let mut parsed = Vec::new();
                for _ in 0..blocks {
                    let Some(h) = take(d, &mut at, 1).map(le) else { return bad("ack block") };
                    let mut a = h >> 4 & 7;
                    if h & 0x80 != 0 {
                        let Some(u) = get_varint(d, &mut at) else { return bad("ack count") };
                        a |= u << 3;
                    }
                    let mut k = h & 7;
                    if h & 0x08 != 0 {
                        let Some(u) = get_varint(d, &mut at) else { return bad("nack count") };
                        k |= u << 3;
                    }
                    parsed.push((a, k));
                }
                let delay_us = (delay != 0xffff).then_some(delay << 5);
                self.apply_ack(latest, &parsed, delay_us, now);
            } else if lead & 0xf8 == 0x88 {
                // Select lane
                let l = lead & 7;
                lane = if l < 7 {
                    l as u32 + 1
                } else {
                    match get_varint(d, &mut at) {
                        Some(v) => v as u32,
                        None => return bad("lane"),
                    }
                };
                cur_unrel = 0;
                rel_pos = 0;
            } else {
                return Err(format!("invalid SNP frame lead byte {lead:#04x}"));
            }
        }
        // Record the packet for our acks.
        self.received.insert(pkt);
        if pkt > self.max_recv {
            self.max_recv = pkt;
            self.max_recv_at = Some(now);
        }
        self.last_recv = Some(now);
        let soon = now + if had_reliable { Duration::from_millis(5) } else { Duration::from_millis(80) };
        self.ack_due = Some(self.ack_due.map_or(soon, |t| t.min(soon)));
        Ok(())
    }

    /// The peer asked for an ack right away (stats flags).
    pub fn ack_now(&mut self, now: Instant) {
        self.ack_due = Some(now);
    }

    pub fn ack_due(&self) -> Option<Instant> {
        self.ack_due
    }

    fn apply_ack(&mut self, latest: u64, blocks: &[(u64, u64)], delay_us: Option<u64>, now: Instant) {
        let mut end = latest + 1;
        let mut i = 0;
        loop {
            let (ack_begin, nack_begin) = if i < blocks.len() {
                let (a, k) = blocks[i];
                let ab = end.saturating_sub(a);
                (ab, ab.saturating_sub(k))
            } else {
                if end <= self.min_waiting {
                    break;
                }
                (self.min_waiting, self.min_waiting)
            };
            let acked: Vec<u64> = self.in_flight.range(ack_begin..end).map(|(n, _)| *n).collect();
            for n in acked {
                self.acked(n, now, n == latest, delay_us);
            }
            let nacked: Vec<u64> = self.in_flight.range(nack_begin..ack_begin).map(|(n, _)| *n).collect();
            for n in nacked {
                self.nack(n);
            }
            end = nack_begin;
            if i >= blocks.len() {
                break;
            }
            i += 1;
        }
        if latest > self.min_waiting {
            self.min_waiting = latest;
        }
        // Records below the settle point are no longer needed once nacked.
        let settled: Vec<u64> = self.in_flight.range(..self.min_waiting).filter(|(_, f)| f.nacked).map(|(n, _)| *n).collect();
        for n in settled {
            self.in_flight.remove(&n);
        }
    }

    fn unreliable_segment(&mut self, num: i64, offset: u32, data: &[u8], last: bool, lane: u32, now: Instant) {
        if offset == 0 && last {
            self.inbox.push_back(Msg { data: data.to_vec(), num, reliable: false, lane });
            return;
        }
        if offset as usize + data.len() > MAX_MESSAGE || self.partials.len() > 64 {
            return;
        }
        let p = self.partials.entry(num).or_default();
        p.since.get_or_insert(now);
        p.parts.insert(offset, data.to_vec());
        if last {
            p.total = Some(offset + data.len() as u32);
        }
        let Some(total) = p.total else { return };
        let mut have = 0u32;
        for (&o, part) in &p.parts {
            if o > have {
                return;
            }
            have = have.max(o + part.len() as u32);
        }
        if have < total {
            return;
        }
        let mut msg = vec![0u8; total as usize];
        for (&o, part) in &p.parts {
            msg[o as usize..o as usize + part.len()].copy_from_slice(part);
        }
        self.partials.remove(&num);
        self.inbox.push_back(Msg { data: msg, num, reliable: false, lane });
    }

    fn reliable_segment(&mut self, begin: u64, data: &[u8], lane: u32) -> Result<(), String> {
        let end = begin + data.len() as u64;
        if end <= self.recv_pos || data.is_empty() {
            return Ok(());
        }
        let have_end = self.recv_pos + self.recv_buf.len() as u64;
        if end - self.recv_pos > MAX_RECV_STREAM as u64 {
            return Ok(()); // too far ahead; they'll resend
        }
        if end > have_end {
            if begin > have_end {
                self.recv_gaps.insert(have_end, begin);
            }
            self.recv_buf.resize((end - self.recv_pos) as usize, 0);
        }
        let (b, skip) = if begin < self.recv_pos { (self.recv_pos, (self.recv_pos - begin) as usize) } else { (begin, 0) };
        // Fill gaps this segment covers.
        let gaps: Vec<(u64, u64)> = self.recv_gaps.iter().filter(|(gb, ge)| **gb < end && **ge > b).map(|(x, y)| (*x, *y)).collect();
        for (gb, ge) in gaps {
            self.recv_gaps.remove(&gb);
            if gb < b {
                self.recv_gaps.insert(gb, b);
            }
            if ge > end {
                self.recv_gaps.insert(end, ge);
            }
        }
        let at = (b - self.recv_pos) as usize;
        self.recv_buf[at..at + data.len() - skip].copy_from_slice(&data[skip..]);
        // Parse whole messages from the front.
        loop {
            let avail = match self.recv_gaps.keys().next() {
                Some(&g) => (g - self.recv_pos) as usize,
                None => self.recv_buf.len(),
            };
            if avail == 0 {
                break;
            }
            let buf = &self.recv_buf[..avail];
            let lead = buf[0];
            if lead & 0x80 != 0 {
                return Err(format!("invalid reliable message header {lead:#04x}"));
            }
            let mut at = 1;
            let mut num = self.last_recv_rel_msg;
            if lead & 0x40 != 0 {
                let Some(o) = get_varint(buf, &mut at) else { break };
                num += o as i64;
            } else {
                num += 1;
            }
            let mut len = (lead & 0x1f) as usize;
            if lead & 0x20 != 0 {
                let Some(u) = get_varint(buf, &mut at) else { break };
                len += (u as usize) << 5;
                if len > MAX_MESSAGE {
                    return Err("reliable message too large".into());
                }
            }
            if at + len > avail {
                break;
            }
            let data = buf[at..at + len].to_vec();
            self.highest_msg = self.highest_msg.max(num);
            self.last_recv_rel_msg = num;
            self.inbox.push_back(Msg { data, num, reliable: true, lane });
            let used = at + len;
            self.recv_buf.drain(..used);
            self.recv_pos += used as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deliver(from: &mut Snp, to: &mut Snp, now: Instant, drop: impl Fn(u64) -> bool) -> usize {
        let mut n = 0;
        while let Some((pkt, body)) = from.build(now, false) {
            n += 1;
            if !drop(pkt) {
                let p = to.expand(pkt as u16).expect("new packet");
                to.receive(p, &body, now).unwrap();
            }
            if n > 1000 {
                panic!("runaway");
            }
        }
        n
    }

    #[test]
    fn varints_and_nearest() {
        let mut v = Vec::new();
        put_varint(&mut v, 300);
        let mut at = 0;
        assert_eq!(get_varint(&v, &mut at), Some(300));
        assert_eq!(nearest(0x0002, 16, 0xfff0), 0x1_0002);
        assert_eq!(nearest(0xfff0, 16, 0x1_0002), 0xfff0);
        assert_eq!(nearest(5, 16, 3), 5);
    }

    #[test]
    fn reliable_and_unreliable_with_loss() {
        let t = Instant::now();
        let (mut a, mut b) = (Snp::new(), Snp::new());
        a.queue(b"one", true);
        a.queue(b"two (unreliable)", false);
        let big: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
        a.queue(&big, true);
        a.queue(&big[..3000], false);
        a.queue(b"three", true);
        // Lose the second packet.
        deliver(&mut a, &mut b, t, |p| p == 3);
        let got: Vec<(bool, usize)> = b.inbox.drain(..).map(|m| (m.reliable, m.data.len())).collect();
        assert_eq!(got[0], (true, 3));
        assert!(got.contains(&(false, 16)));
        // The big reliable one waits for the lost segment.
        assert!(!got.contains(&(true, 5000)));
        // b acks with a nack block; a resends.
        deliver(&mut b, &mut a, t, |_| false);
        deliver(&mut a, &mut b, t, |_| false);
        let got: Vec<Msg> = b.inbox.drain(..).collect();
        assert_eq!(got.iter().filter(|m| m.reliable).map(|m| m.data.len()).collect::<Vec<_>>(), vec![5000, 5]);
        assert_eq!(got.iter().find(|m| m.data.len() == 5000).unwrap().data, big);
        assert_eq!(got.iter().find(|m| m.data.len() == 5).unwrap().num, 5);
        // All acked eventually.
        deliver(&mut b, &mut a, t, |_| false);
        assert!(!a.has_unacked());
    }

    #[test]
    fn timeouts_resend() {
        let t = Instant::now();
        let (mut a, mut b) = (Snp::new(), Snp::new());
        a.queue(b"x", true);
        deliver(&mut a, &mut b, t, |_| true);
        a.tick(t + Duration::from_secs(1));
        deliver(&mut a, &mut b, t + Duration::from_secs(1), |_| false);
        assert_eq!(b.inbox.pop_front().unwrap().data, b"x");
    }
}

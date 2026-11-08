//! PseudoTcp: the reliable stream Valve's legacy P2P (libjingle 0.4.0)
//! runs inside a UDP connection. A port of `talk/p2p/base/pseudotcp.cc`
//! from libjingle 0.4.0, which Steam's client still ships for
//! ISteamNetworking. It is wire-compatible with it:
//!
//! ```text
//! conv u32 | seq u32 | ack u32 | 0 u8 | flags u8 | wnd u16 | tsval u32 | tsecr u32 | data
//! ```
//! all big-endian. The first segment carries the control byte CTL_CONNECT
//! (flag CTL), which takes one byte of sequence space. Sequence numbers
//! start at 0 and aren't compared modulo 2^32, like the original.

use std::collections::VecDeque;

pub const HEADER_SIZE: usize = 24;
const MIN_RTO: u32 = 250;
const DEF_RTO: u32 = 3000;
const MAX_RTO: u32 = 60000;
const ACK_DELAY: u32 = 100;
const FLAG_CTL: u8 = 0x02;
const FLAG_RST: u8 = 0x04;
const CTL_CONNECT: u8 = 0;
/// Receive buffer (and so the largest window we advertise); Valve's is 60 KB too.
const RBUF: usize = 60 * 1024;
const SBUF: usize = 90 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum State {
    Listen,
    SynSent,
    SynReceived,
    Established,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SendFlags {
    None,
    DelayedAck,
    ImmediateAck,
}

struct SSeg {
    seq: u32,
    len: u32,
    xmit: u8,
    ctrl: bool,
}

struct RSeg {
    seq: u32,
    len: u32,
}

/// Signed difference `later - earlier` of two ms clocks.
fn diff(later: u32, earlier: u32) -> i32 {
    later.wrapping_sub(earlier) as i32
}

pub struct PseudoTcp {
    conv: u32,
    pub state: State,
    /// Why it closed (an errno-like code), once closed.
    pub error: u32,
    sbuf: Vec<u8>,
    slen: u32,
    slist: VecDeque<SSeg>,
    rbuf: Vec<u8>,
    rlen: u32,
    rlist: Vec<RSeg>,
    snd_nxt: u32,
    snd_wnd: u32,
    snd_una: u32,
    rcv_nxt: u32,
    rcv_wnd: u32,
    mss: u32,
    cwnd: u32,
    ssthresh: u32,
    dup_acks: u32,
    recover: u32,
    ts_recent: u32,
    ts_lastack: u32,
    rx_rto: u32,
    rx_srtt: u32,
    rx_rttvar: u32,
    rto_base: u32,
    t_ack: u32,
    lastsend: u32,
    lastrecv: u32,
    /// Segments ready for the wire (header + data).
    out: Vec<Vec<u8>>,
    /// Opened since the last [`take_opened`](Self::take_opened).
    opened: bool,
}

impl PseudoTcp {
    /// `mss`: the largest payload per segment we send.
    pub fn new(conv: u32, mss: u32, now: u32) -> Self {
        PseudoTcp {
            conv,
            state: State::Listen,
            error: 0,
            sbuf: vec![0; SBUF],
            slen: 0,
            slist: VecDeque::new(),
            rbuf: vec![0; RBUF],
            rlen: 0,
            rlist: Vec::new(),
            snd_nxt: 0,
            snd_wnd: 1,
            snd_una: 0,
            rcv_nxt: 0,
            rcv_wnd: RBUF as u32,
            mss,
            cwnd: 2 * mss,
            ssthresh: RBUF as u32,
            dup_acks: 0,
            recover: 0,
            ts_recent: 0,
            ts_lastack: 0,
            rx_rto: DEF_RTO,
            rx_srtt: 0,
            rx_rttvar: 0,
            rto_base: 0,
            t_ack: 0,
            lastsend: now,
            lastrecv: now,
            out: Vec::new(),
            opened: false,
        }
    }

    pub fn connect(&mut self, now: u32) {
        if self.state != State::Listen {
            return;
        }
        self.state = State::SynSent;
        self.queue(&[CTL_CONNECT], true);
        self.attempt_send(SendFlags::None, now);
    }

    pub fn take_output(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.out)
    }

    /// True once after the stream opened.
    pub fn take_opened(&mut self) -> bool {
        std::mem::take(&mut self.opened)
    }

    /// Room for more stream bytes.
    pub fn send_space(&self) -> usize {
        SBUF - self.slen as usize
    }

    /// Queue stream bytes; returns how many were taken.
    pub fn send(&mut self, data: &[u8], now: u32) -> usize {
        if self.state != State::Established {
            return 0;
        }
        let n = self.queue(data, false);
        self.attempt_send(SendFlags::None, now);
        n as usize
    }

    /// Everything received in order so far.
    pub fn recv(&mut self, out: &mut Vec<u8>, now: u32) {
        if self.rlen == 0 {
            return;
        }
        let n = self.rlen as usize;
        out.extend_from_slice(&self.rbuf[..n]);
        self.rbuf.copy_within(n.., 0);
        self.rlen = 0;
        if (RBUF as u32 - self.rlen - self.rcv_wnd) >= (RBUF as u32 / 2).min(self.mss) {
            let was_closed = self.rcv_wnd == 0;
            self.rcv_wnd = RBUF as u32 - self.rlen;
            if was_closed {
                self.attempt_send(SendFlags::ImmediateAck, now);
            }
        }
    }

    pub fn close(&mut self) {
        self.slen = 0;
        self.state = State::Closed;
    }

    fn queue(&mut self, data: &[u8], ctrl: bool) -> u32 {
        let len = (data.len() as u32).min(SBUF as u32 - self.slen);
        if let Some(back) = self.slist.back_mut().filter(|b| b.ctrl == ctrl && b.xmit == 0) {
            back.len += len;
        } else {
            self.slist.push_back(SSeg { seq: self.snd_una.wrapping_add(self.slen), len, xmit: 0, ctrl });
        }
        let at = self.slen as usize;
        self.sbuf[at..at + len as usize].copy_from_slice(&data[..len as usize]);
        self.slen += len;
        len
    }

    fn packet(&mut self, seq: u32, flags: u8, offset: usize, len: u32, now: u32) {
        let mut b = Vec::with_capacity(HEADER_SIZE + len as usize);
        b.extend_from_slice(&self.conv.to_be_bytes());
        b.extend_from_slice(&seq.to_be_bytes());
        b.extend_from_slice(&self.rcv_nxt.to_be_bytes());
        b.push(0);
        b.push(flags);
        b.extend_from_slice(&(self.rcv_wnd.min(0xFFFF) as u16).to_be_bytes());
        b.extend_from_slice(&now.to_be_bytes());
        b.extend_from_slice(&self.ts_recent.to_be_bytes());
        self.ts_lastack = self.rcv_nxt;
        b.extend_from_slice(&self.sbuf[offset..offset + len as usize]);
        self.out.push(b);
        self.t_ack = 0;
        if len > 0 {
            self.lastsend = now;
        }
    }

    fn ack(&mut self, now: u32) {
        let seq = self.snd_nxt;
        self.packet(seq, 0, 0, 0, now);
    }

    /// Timers: retransmission, window probes, delayed acks. Call often.
    pub fn notify_clock(&mut self, now: u32) {
        if self.state == State::Closed {
            return;
        }
        if self.rto_base != 0 && diff(self.rto_base.wrapping_add(self.rx_rto), now) <= 0 {
            if self.slist.is_empty() {
                self.rto_base = 0;
            } else {
                if !self.transmit(0, now) {
                    self.closedown(103); // ECONNABORTED
                    return;
                }
                let in_flight = self.snd_nxt.wrapping_sub(self.snd_una);
                self.ssthresh = (in_flight / 2).max(2 * self.mss);
                self.cwnd = self.mss;
                let limit = if self.state < State::Established { DEF_RTO } else { MAX_RTO };
                self.rx_rto = limit.min(self.rx_rto * 2);
                self.rto_base = now;
            }
        }
        if self.snd_wnd == 0 && diff(self.lastsend.wrapping_add(self.rx_rto), now) <= 0 {
            if diff(now, self.lastrecv) >= 15000 {
                self.closedown(103);
                return;
            }
            let seq = self.snd_nxt.wrapping_sub(1);
            self.packet(seq, 0, 0, 0, now);
            self.lastsend = now;
            self.rx_rto = MAX_RTO.min(self.rx_rto * 2);
        }
        if self.t_ack != 0 && diff(self.t_ack.wrapping_add(ACK_DELAY), now) <= 0 {
            self.ack(now);
        }
    }

    /// A segment from the peer. False if it was rejected.
    pub fn notify_packet(&mut self, buf: &[u8], now: u32) -> bool {
        if buf.len() < HEADER_SIZE {
            return false;
        }
        let be32 = |at: usize| u32::from_be_bytes(buf[at..at + 4].try_into().unwrap());
        let conv = be32(0);
        let seq = be32(4);
        let ack = be32(8);
        let flags = buf[13];
        let wnd = u16::from_be_bytes([buf[14], buf[15]]) as u32;
        let tsval = be32(16);
        let tsecr = be32(20);
        self.process(conv, seq, ack, flags, wnd, tsval, tsecr, &buf[HEADER_SIZE..], now)
    }

    #[allow(clippy::too_many_arguments)]
    fn process(&mut self, conv: u32, mut seq: u32, ack: u32, flags: u8, wnd: u32, tsval: u32, tsecr: u32, data: &[u8], now: u32) -> bool {
        if conv != self.conv || self.state == State::Closed {
            return false;
        }
        self.lastrecv = now;
        if flags & FLAG_RST != 0 {
            self.closedown(104); // ECONNRESET
            return false;
        }
        let mut data = data;
        let mut len = data.len() as u32;
        let mut connect = false;
        if flags & FLAG_CTL != 0 {
            if data.is_empty() || data[0] != CTL_CONNECT {
                return false;
            }
            connect = true;
            if self.state == State::Listen {
                self.state = State::SynReceived;
                self.queue(&[CTL_CONNECT], true);
            } else if self.state == State::SynSent {
                self.established();
            }
        }
        // Timestamp to echo.
        if seq <= self.ts_lastack && self.ts_lastack < seq.wrapping_add(len) {
            self.ts_recent = tsval;
        }
        if ack > self.snd_una && ack <= self.snd_nxt {
            if tsecr != 0 {
                let rtt = diff(now, tsecr);
                if rtt >= 0 {
                    let rtt = rtt as u32;
                    if self.rx_srtt == 0 {
                        self.rx_srtt = rtt;
                        self.rx_rttvar = rtt / 2;
                    } else {
                        self.rx_rttvar = (3 * self.rx_rttvar + (rtt as i64 - self.rx_srtt as i64).unsigned_abs() as u32) / 4;
                        self.rx_srtt = (7 * self.rx_srtt + rtt) / 8;
                    }
                    self.rx_rto = (self.rx_srtt + (4 * self.rx_rttvar).max(1)).clamp(MIN_RTO, MAX_RTO);
                }
            }
            self.snd_wnd = wnd;
            let acked = ack - self.snd_una;
            self.snd_una = ack;
            self.rto_base = if self.snd_una == self.snd_nxt { 0 } else { now };
            self.slen -= acked;
            let s = self.slen as usize;
            self.sbuf.copy_within(acked as usize..acked as usize + s, 0);
            let mut free = acked;
            while free > 0 {
                let Some(front) = self.slist.front_mut() else { break };
                if free < front.len {
                    front.len -= free;
                    front.seq = front.seq.wrapping_add(free);
                    free = 0;
                } else {
                    free -= front.len;
                    self.slist.pop_front();
                }
            }
            if self.dup_acks >= 3 {
                if self.snd_una >= self.recover {
                    let in_flight = self.snd_nxt - self.snd_una;
                    self.cwnd = self.ssthresh.min(in_flight + self.mss);
                    self.dup_acks = 0;
                } else {
                    if !self.transmit(0, now) {
                        self.closedown(103);
                        return false;
                    }
                    self.cwnd = self.cwnd - acked.min(self.cwnd) + self.mss;
                }
            } else {
                self.dup_acks = 0;
                if self.cwnd < self.ssthresh {
                    self.cwnd += self.mss;
                } else {
                    self.cwnd += (self.mss * self.mss / self.cwnd).max(1);
                }
            }
            if self.state == State::SynReceived && !connect {
                self.established();
            }
        } else if ack == self.snd_una {
            self.snd_wnd = wnd;
            if len > 0 {
                // Data with a duplicate ack: not a dup ack.
            } else if self.snd_una != self.snd_nxt {
                self.dup_acks += 1;
                if self.dup_acks == 3 {
                    if !self.transmit(0, now) {
                        self.closedown(103);
                        return false;
                    }
                    self.recover = self.snd_nxt;
                    let in_flight = self.snd_nxt - self.snd_una;
                    self.ssthresh = (in_flight / 2).max(2 * self.mss);
                    self.cwnd = self.ssthresh + 3 * self.mss;
                } else if self.dup_acks > 3 {
                    self.cwnd += self.mss;
                }
            } else {
                self.dup_acks = 0;
            }
        }

        let mut sflags = if seq != self.rcv_nxt {
            SendFlags::ImmediateAck
        } else if len != 0 {
            SendFlags::DelayedAck
        } else {
            SendFlags::None
        };
        // Trim what we already have.
        if seq < self.rcv_nxt {
            let adjust = self.rcv_nxt - seq;
            if adjust < len {
                seq += adjust;
                data = &data[adjust as usize..];
                len -= adjust;
            } else {
                len = 0;
            }
        }
        // And what doesn't fit.
        let room = RBUF as u32 - self.rlen;
        if seq.wrapping_add(len).wrapping_sub(self.rcv_nxt) > room {
            let adjust = seq.wrapping_add(len).wrapping_sub(self.rcv_nxt) - room;
            len = len.saturating_sub(adjust);
        }
        let ignore = flags & FLAG_CTL != 0;
        if len > 0 {
            if ignore {
                if seq == self.rcv_nxt {
                    self.rcv_nxt += len;
                }
            } else {
                let off = (self.rlen + (seq - self.rcv_nxt)) as usize;
                self.rbuf[off..off + len as usize].copy_from_slice(&data[..len as usize]);
                if seq == self.rcv_nxt {
                    self.rlen += len;
                    self.rcv_nxt += len;
                    self.rcv_wnd -= len.min(self.rcv_wnd);
                    // Earlier out-of-order segments that now fit.
                    while let Some(r) = self.rlist.first().filter(|r| r.seq <= self.rcv_nxt) {
                        if r.seq + r.len > self.rcv_nxt {
                            sflags = SendFlags::ImmediateAck;
                            let adjust = r.seq + r.len - self.rcv_nxt;
                            self.rlen += adjust;
                            self.rcv_nxt += adjust;
                            self.rcv_wnd -= adjust.min(self.rcv_wnd);
                        }
                        self.rlist.remove(0);
                    }
                } else {
                    let pos = self.rlist.iter().position(|r| r.seq >= seq).unwrap_or(self.rlist.len());
                    self.rlist.insert(pos, RSeg { seq, len });
                }
            }
        }
        self.attempt_send(sflags, now);
        true
    }

    fn established(&mut self) {
        self.state = State::Established;
        self.ssthresh = self.ssthresh.max(2 * self.mss);
        self.cwnd = self.cwnd.max(self.mss);
        self.opened = true;
    }

    /// (Re)send segment `idx` of the send list.
    fn transmit(&mut self, idx: usize, now: u32) -> bool {
        let limit = if self.state == State::Established { 15 } else { 30 };
        let (seq, ctrl, xmit, seg_len) = {
            let s = &self.slist[idx];
            (s.seq, s.ctrl, s.xmit, s.len)
        };
        if xmit >= limit {
            return false;
        }
        let n = seg_len.min(self.mss);
        let offset = seq.wrapping_sub(self.snd_una) as usize;
        self.packet(seq, if ctrl { FLAG_CTL } else { 0 }, offset, n, now);
        if n < seg_len {
            self.slist[idx].len = n;
            self.slist.insert(idx + 1, SSeg { seq: seq.wrapping_add(n), len: seg_len - n, xmit, ctrl });
        }
        let s = &mut self.slist[idx];
        if s.xmit == 0 {
            self.snd_nxt = self.snd_nxt.wrapping_add(s.len);
        }
        s.xmit += 1;
        if self.rto_base == 0 {
            self.rto_base = now;
        }
        true
    }

    fn attempt_send(&mut self, mut sflags: SendFlags, now: u32) {
        if diff(now, self.lastsend) > self.rx_rto as i32 {
            self.cwnd = self.mss;
        }
        loop {
            let mut cwnd = self.cwnd;
            if self.dup_acks == 1 || self.dup_acks == 2 {
                cwnd += self.dup_acks * self.mss;
            }
            let window = self.snd_wnd.min(cwnd);
            let in_flight = self.snd_nxt.wrapping_sub(self.snd_una);
            let useable = window.saturating_sub(in_flight);
            let mut available = (self.slen - in_flight).min(self.mss);
            if available > useable {
                available = if useable * 4 < window { 0 } else { useable };
            }
            if available == 0 {
                if sflags == SendFlags::None {
                    return;
                }
                if sflags == SendFlags::ImmediateAck || self.t_ack != 0 {
                    self.ack(now);
                } else {
                    self.t_ack = now.max(1);
                }
                return;
            }
            // Nagle: wait while data is in flight and the segment isn't full.
            if self.snd_nxt > self.snd_una && available < self.mss {
                return;
            }
            let Some(idx) = self.slist.iter().position(|s| s.xmit == 0) else { return };
            if self.slist[idx].len > available {
                let s = &self.slist[idx];
                let sub = SSeg { seq: s.seq.wrapping_add(available), len: s.len - available, xmit: 0, ctrl: s.ctrl };
                self.slist[idx].len = available;
                self.slist.insert(idx + 1, sub);
            }
            if !self.transmit(idx, now) {
                return;
            }
            sflags = SendFlags::None;
        }
    }

    fn closedown(&mut self, err: u32) {
        self.slen = 0;
        self.state = State::Closed;
        self.error = err;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two ends with a lossy wire; checks the stream arrives whole and in order.
    fn pump(a: &mut PseudoTcp, b: &mut PseudoTcp, now: u32, drop_every: usize, n: &mut usize) {
        for seg in a.take_output() {
            *n += 1;
            if drop_every == 0 || *n % drop_every != 0 {
                b.notify_packet(&seg, now);
            }
        }
        for seg in b.take_output() {
            *n += 1;
            if drop_every == 0 || *n % drop_every != 0 {
                a.notify_packet(&seg, now);
            }
        }
    }

    fn run(drop_every: usize, both_connect: bool) {
        let mut a = PseudoTcp::new(0, 1100, 1000);
        let mut b = PseudoTcp::new(0, 1100, 1000);
        a.connect(1000);
        if both_connect {
            b.connect(1000);
        }
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let mut sent = 0;
        let mut got = Vec::new();
        let mut n = 0;
        let mut now = 1000;
        while got.len() < data.len() && now < 1000 + 600_000 {
            now += 10;
            if a.state == State::Established && sent < data.len() {
                sent += a.send(&data[sent..], now);
            }
            pump(&mut a, &mut b, now, drop_every, &mut n);
            a.notify_clock(now);
            b.notify_clock(now);
            b.recv(&mut got, now);
        }
        assert_eq!(b.state, State::Established);
        assert_eq!(got.len(), data.len(), "all bytes arrive");
        assert!(got == data, "in order and intact");
    }

    #[test]
    fn stream_over_clean_wire() {
        run(0, false);
    }

    #[test]
    fn stream_with_loss_and_simultaneous_open() {
        run(7, true);
    }

    #[test]
    fn header_layout() {
        let mut a = PseudoTcp::new(0, 180, 5000);
        a.connect(5000);
        let out = a.take_output();
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.len(), HEADER_SIZE + 1);
        assert_eq!(&s[0..4], &[0, 0, 0, 0], "conversation 0");
        assert_eq!(&s[4..8], &[0, 0, 0, 0], "seq 0");
        assert_eq!(s[13], FLAG_CTL);
        assert_eq!(u16::from_be_bytes([s[14], s[15]]) as usize, RBUF);
        assert_eq!(u32::from_be_bytes(s[16..20].try_into().unwrap()), 5000);
        assert_eq!(s[24], CTL_CONNECT);
    }
}

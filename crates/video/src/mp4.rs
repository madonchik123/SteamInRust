//! A small ISO-BMFF (MP4) reader: just enough for H.264 + AAC in plain MP4
//! files (Steam's microtrailers) and fragmented MP4 segments (Steam's HLS and
//! DASH trailers).

#[derive(Debug, Clone, PartialEq)]
pub enum Codec {
    H264 { nal_len: usize, sps: Vec<Vec<u8>>, pps: Vec<Vec<u8>> },
    Aac { config: Vec<u8>, channels: u16, rate: u32 },
    Other([u8; 4]),
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: u32,
    pub timescale: u32,
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    /// Media time where presentation starts (edit list), in track units.
    pub delay: i64,
    /// Duration from the media header, in track units (0 for fragmented files).
    pub duration: u64,
    /// Sample table of a plain MP4; empty for fragmented files.
    pub samples: Vec<Sample>,
    default_duration: u32,
    default_size: u32,
    default_flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Byte range in the file / segment.
    pub offset: usize,
    pub size: usize,
    /// Decode time, in track units.
    pub dts: i64,
    /// Composition offset, in track units.
    pub cts: i64,
    pub duration: u32,
    pub key: bool,
}

impl Track {
    pub fn is_video(&self) -> bool {
        matches!(self.codec, Codec::H264 { .. })
    }
    pub fn is_audio(&self) -> bool {
        matches!(self.codec, Codec::Aac { .. })
    }
    /// Presentation time of a sample in seconds.
    pub fn pts(&self, s: &Sample) -> f64 {
        (s.dts + s.cts - self.delay) as f64 / self.timescale.max(1) as f64
    }
    pub fn seconds(&self, units: i64) -> f64 {
        units as f64 / self.timescale.max(1) as f64
    }
}

#[derive(Debug, Clone, Default)]
pub struct Movie {
    pub tracks: Vec<Track>,
}

impl Movie {
    pub fn video(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_video())
    }
    pub fn audio(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_audio())
    }
}

/// Big-endian reader over a byte slice that never panics.
#[derive(Clone, Copy)]
struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, p: 0 }
    }
    fn left(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(|_| ())
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.bytes(8)?.try_into().ok()?))
    }
    /// Version-dependent 32/64-bit field.
    fn uv(&mut self, v: u8) -> Option<u64> {
        if v == 1 { self.u64() } else { self.u32().map(u64::from) }
    }
}

/// Iterate the boxes in `d`: (type, payload, absolute offset of the box start).
fn boxes(d: &[u8], base: usize) -> impl Iterator<Item = ([u8; 4], &[u8], usize)> {
    let mut p = 0usize;
    std::iter::from_fn(move || {
        let mut r = R::new(d.get(p..)?);
        let size = r.u32()? as u64;
        let kind: [u8; 4] = r.bytes(4)?.try_into().ok()?;
        let (size, header) = match size {
            1 => (r.u64()?, 16u64),
            0 => ((d.len() - p) as u64, 8),
            s => (s, 8),
        };
        if size < header {
            return None;
        }
        // A box cut short (a partial download) still yields what is there.
        let end = (p as u64 + size).min(d.len() as u64) as usize;
        let payload = d.get(p + header as usize..end)?;
        let at = base + p;
        p = end;
        Some((kind, payload, at))
    })
}

fn child<'a>(d: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(d, 0).find(|(k, _, _)| k == kind).map(|(_, p, _)| p)
}

/// Parse the `moov` of an init segment or a whole MP4 file.
pub fn parse_movie(data: &[u8]) -> Result<Movie, String> {
    let moov = child(data, b"moov").ok_or("no moov box")?;
    let movie_scale = child(moov, b"mvhd")
        .and_then(|m| {
            let mut r = R::new(m);
            let v = r.u8()?;
            r.skip(3)?;
            r.uv(v)?;
            r.uv(v)?;
            r.u32()
        })
        .unwrap_or(1000);
    let trex: Vec<(u32, u32, u32, u32)> = child(moov, b"mvex")
        .map(|mvex| {
            boxes(mvex, 0)
                .filter(|(k, _, _)| k == b"trex")
                .filter_map(|(_, p, _)| {
                    let mut r = R::new(p);
                    r.skip(4)?;
                    let id = r.u32()?;
                    r.skip(4)?;
                    Some((id, r.u32()?, r.u32()?, r.u32()?))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut tracks = Vec::new();
    for (k, trak, _) in boxes(moov, 0) {
        if &k != b"trak" {
            continue;
        }
        if let Some(mut t) = parse_trak(trak, movie_scale) {
            if let Some(&(_, dur, size, flags)) = trex.iter().find(|x| x.0 == t.id) {
                t.default_duration = dur;
                t.default_size = size;
                t.default_flags = flags;
            }
            tracks.push(t);
        }
    }
    Ok(Movie { tracks })
}

fn parse_trak(trak: &[u8], movie_scale: u32) -> Option<Track> {
    let tkhd = child(trak, b"tkhd")?;
    let id = {
        let mut r = R::new(tkhd);
        let v = r.u8()?;
        r.skip(3)?;
        r.uv(v)?;
        r.uv(v)?;
        r.u32()?
    };
    let mdia = child(trak, b"mdia")?;
    let (timescale, duration) = {
        let mut r = R::new(child(mdia, b"mdhd")?);
        let v = r.u8()?;
        r.skip(3)?;
        r.uv(v)?;
        r.uv(v)?;
        (r.u32()?, r.uv(v)?)
    };
    let stbl = child(child(mdia, b"minf")?, b"stbl")?;
    let (codec, width, height) = parse_stsd(child(stbl, b"stsd")?)?;
    let delay = child(trak, b"edts").and_then(|e| child(e, b"elst")).and_then(|e| parse_elst(e, movie_scale, timescale)).unwrap_or(0);
    let samples = parse_stbl(stbl).unwrap_or_default();
    Some(Track { id, timescale, codec, width, height, delay, duration: if duration == u64::MAX { 0 } else { duration }, samples, default_duration: 0, default_size: 0, default_flags: 0 })
}

/// Start of presentation in media units: the first real edit's media time,
/// minus any leading empty edit (a delay).
fn parse_elst(e: &[u8], movie_scale: u32, timescale: u32) -> Option<i64> {
    let mut r = R::new(e);
    let v = r.u8()?;
    r.skip(3)?;
    let n = r.u32()?;
    let mut empty = 0i64;
    for _ in 0..n {
        let (dur, media) = if v == 1 { (r.u64()? as i64, r.u64()? as i64) } else { (r.u32()? as i64, r.u32()? as i32 as i64) };
        r.skip(4)?;
        if media == -1 {
            empty += dur * timescale as i64 / movie_scale.max(1) as i64;
        } else {
            return Some(media - empty);
        }
    }
    None
}

fn parse_stsd(stsd: &[u8]) -> Option<(Codec, u32, u32)> {
    let mut r = R::new(stsd);
    r.skip(8)?; // version/flags, entry count
    let (kind, entry, _) = boxes(&stsd[r.p..], 0).next()?;
    match &kind {
        b"avc1" | b"avc3" => {
            let mut e = R::new(entry);
            e.skip(6 + 2 + 16)?;
            let w = e.u16()? as u32;
            let h = e.u16()? as u32;
            e.skip(4 + 4 + 4 + 2 + 32 + 2 + 2)?;
            let avcc = child(&entry[e.p..], b"avcC")?;
            Some((parse_avcc(avcc)?, w, h))
        }
        b"mp4a" => {
            let mut e = R::new(entry);
            e.skip(6 + 2 + 8)?;
            let channels = e.u16()?;
            e.skip(2 + 2 + 2)?;
            let rate = e.u32()? >> 16;
            let esds = child(&entry[e.p..], b"esds")?;
            let config = parse_esds(esds).unwrap_or_default();
            Some((Codec::Aac { config, channels, rate }, 0, 0))
        }
        other => Some((Codec::Other(*other), 0, 0)),
    }
}

fn parse_avcc(d: &[u8]) -> Option<Codec> {
    let mut r = R::new(d);
    r.skip(4)?;
    let nal_len = (r.u8()? & 3) as usize + 1;
    let n_sps = r.u8()? & 0x1f;
    let mut sps = Vec::new();
    for _ in 0..n_sps {
        let n = r.u16()? as usize;
        sps.push(r.bytes(n)?.to_vec());
    }
    let n_pps = r.u8()?;
    let mut pps = Vec::new();
    for _ in 0..n_pps {
        let n = r.u16()? as usize;
        pps.push(r.bytes(n)?.to_vec());
    }
    Some(Codec::H264 { nal_len, sps, pps })
}

/// The AudioSpecificConfig inside an `esds` box.
fn parse_esds(d: &[u8]) -> Option<Vec<u8>> {
    let mut r = R::new(d);
    r.skip(4)?;
    fn len(r: &mut R) -> Option<usize> {
        let mut n = 0usize;
        for _ in 0..4 {
            let b = r.u8()?;
            n = (n << 7) | (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        Some(n)
    }
    while r.left() > 2 {
        let tag = r.u8()?;
        let n = len(&mut r)?;
        match tag {
            0x03 => {
                r.skip(2)?;
                let flags = r.u8()?;
                if flags & 0x80 != 0 {
                    r.skip(2)?;
                }
                if flags & 0x40 != 0 {
                    let l = r.u8()? as usize;
                    r.skip(l)?;
                }
                if flags & 0x20 != 0 {
                    r.skip(2)?;
                }
            }
            0x04 => r.skip(13)?,
            0x05 => return Some(r.bytes(n)?.to_vec()),
            _ => r.skip(n)?,
        }
    }
    None
}

/// The full sample table of a plain MP4 track.
fn parse_stbl(stbl: &[u8]) -> Option<Vec<Sample>> {
    let full = |k: &[u8; 4]| child(stbl, k).map(|b| (b.first().copied().unwrap_or(0), R::new(b.get(4..).unwrap_or_default())));
    // Sizes.
    let (_, mut r) = full(b"stsz")?;
    let fixed = r.u32()?;
    let count = r.u32()? as usize;
    if count == 0 {
        return None;
    }
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        sizes.push(if fixed != 0 { fixed } else { r.u32()? });
    }
    // Chunk offsets.
    let offsets: Vec<u64> = if let Some((_, mut r)) = full(b"stco") {
        let n = r.u32()?;
        (0..n).map(|_| r.u32().map(u64::from)).collect::<Option<_>>()?
    } else {
        let (_, mut r) = full(b"co64")?;
        let n = r.u32()?;
        (0..n).map(|_| r.u64()).collect::<Option<_>>()?
    };
    // Samples per chunk.
    let (_, mut r) = full(b"stsc")?;
    let n = r.u32()?;
    let mut stsc = Vec::new();
    for _ in 0..n {
        let first = r.u32()?;
        let per = r.u32()?;
        r.skip(4)?;
        stsc.push((first, per));
    }
    // Durations.
    let (_, mut r) = full(b"stts")?;
    let n = r.u32()?;
    let mut durs = Vec::with_capacity(count);
    for _ in 0..n {
        let c = r.u32()?;
        let d = r.u32()?;
        durs.extend(std::iter::repeat_n(d, c as usize));
    }
    // Composition offsets.
    let mut ctts = Vec::new();
    if let Some((v, mut r)) = full(b"ctts") {
        let n = r.u32()?;
        for _ in 0..n {
            let c = r.u32()?;
            let o = r.u32()?;
            let o = if v == 1 { o as i32 as i64 } else { o as i64 };
            ctts.extend(std::iter::repeat_n(o, c as usize));
        }
    }
    // Sync samples (1-based); none listed means every sample is a key frame.
    let sync: Option<Vec<u32>> = full(b"stss").and_then(|(_, mut r)| {
        let n = r.u32()?;
        (0..n).map(|_| r.u32()).collect()
    });

    let mut out = Vec::with_capacity(count);
    let mut dts = 0i64;
    let mut idx = 0usize;
    for (ci, &off) in offsets.iter().enumerate() {
        let chunk = ci as u32 + 1;
        let per = stsc.iter().rev().find(|(f, _)| *f <= chunk).map(|x| x.1).unwrap_or(1);
        let mut pos = off as usize;
        for _ in 0..per {
            if idx >= count {
                break;
            }
            let size = sizes[idx] as usize;
            let duration = durs.get(idx).copied().unwrap_or(0);
            let key = sync.as_ref().is_none_or(|s| s.binary_search(&(idx as u32 + 1)).is_ok());
            out.push(Sample { offset: pos, size, dts, cts: ctts.get(idx).copied().unwrap_or(0), duration, key });
            pos += size;
            dts += duration as i64;
            idx += 1;
        }
    }
    Some(out)
}

/// The samples of one track inside a fragmented MP4 segment (`moof` + `mdat`
/// pairs). Offsets point into `data`.
pub fn parse_fragment(data: &[u8], track: &Track) -> Vec<Sample> {
    let mut out = Vec::new();
    for (k, moof, moof_at) in boxes(data, 0) {
        if &k != b"moof" {
            continue;
        }
        for (k, traf, _) in boxes(moof, 0) {
            if &k == b"traf" {
                parse_traf(traf, moof_at, track, &mut out);
            }
        }
    }
    out
}

fn parse_traf(traf: &[u8], moof_at: usize, t: &Track, out: &mut Vec<Sample>) -> Option<()> {
    let mut r = R::new(child(traf, b"tfhd")?);
    let flags = r.u32()? & 0xff_ffff;
    if r.u32()? != t.id {
        return None;
    }
    let mut base = moof_at as u64;
    if flags & 0x1 != 0 {
        base = r.u64()?;
    }
    if flags & 0x2 != 0 {
        r.skip(4)?;
    }
    let def_dur = if flags & 0x8 != 0 { r.u32()? } else { t.default_duration };
    let def_size = if flags & 0x10 != 0 { r.u32()? } else { t.default_size };
    let def_flags = if flags & 0x20 != 0 { r.u32()? } else { t.default_flags };
    let mut dts = child(traf, b"tfdt")
        .and_then(|b| {
            let mut r = R::new(b);
            let v = r.u8()?;
            r.skip(3)?;
            r.uv(v)
        })
        .unwrap_or(0) as i64;
    for (k, trun, _) in boxes(traf, 0) {
        if &k != b"trun" {
            continue;
        }
        let mut r = R::new(trun);
        let v = r.u8()?;
        let f = {
            let b = r.bytes(3)?;
            (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32
        };
        let n = r.u32()?;
        let mut pos = base as i64;
        if f & 0x1 != 0 {
            pos += r.u32()? as i32 as i64;
        }
        let first_flags = if f & 0x4 != 0 { Some(r.u32()?) } else { None };
        for i in 0..n {
            let duration = if f & 0x100 != 0 { r.u32()? } else { def_dur };
            let size = if f & 0x200 != 0 { r.u32()? } else { def_size };
            let sflags = if f & 0x400 != 0 { r.u32()? } else if i == 0 { first_flags.unwrap_or(def_flags) } else { def_flags };
            let cts = if f & 0x800 != 0 {
                let c = r.u32()?;
                if v == 0 { c as i64 } else { c as i32 as i64 }
            } else {
                0
            };
            out.push(Sample { offset: pos.max(0) as usize, size: size as usize, dts, cts, duration, key: sflags & 0x1_0000 == 0 });
            pos += size as i64;
            dts += duration as i64;
        }
    }
    Some(())
}

/// Length-prefixed NAL units (MP4) to an Annex B byte stream (start codes),
/// with the parameter sets in front of key frames.
pub fn annex_b(sample: &[u8], nal_len: usize, params: Option<(&[Vec<u8>], &[Vec<u8>])>, out: &mut Vec<u8>) {
    out.clear();
    if let Some((sps, pps)) = params {
        for p in sps.iter().chain(pps) {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(p);
        }
    }
    let mut p = 0usize;
    while p + nal_len <= sample.len() {
        let mut n = 0usize;
        for b in &sample[p..p + nal_len] {
            n = (n << 8) | *b as usize;
        }
        p += nal_len;
        let end = (p + n).min(sample.len());
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&sample[p..end]);
        p = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn annex_b_rewrites_lengths() {
        let mut out = Vec::new();
        annex_b(&[0, 0, 0, 2, 0x65, 1, 0, 0, 0, 1, 0x41], 4, Some((&[vec![0x67]], &[vec![0x68]])), &mut out);
        assert_eq!(out, [0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68, 0, 0, 0, 1, 0x65, 1, 0, 0, 0, 1, 0x41]);
    }

    #[test]
    fn fragment_samples() {
        // trun with duration, size, flags and signed composition offsets.
        let tfhd = bx(b"tfhd", &[&0x02_0000u32.to_be_bytes()[..], &1u32.to_be_bytes()].concat());
        let tfdt = bx(b"tfdt", &[&[1u8, 0, 0, 0][..], &3000u64.to_be_bytes()].concat());
        let mut trun = vec![1u8, 0, 0x0f, 0x01];
        trun.extend(2u32.to_be_bytes());
        trun.extend(0u32.to_be_bytes()); // data offset, patched below
        for (d, s, f, c) in [(1000u32, 5u32, 0u32, 2000i32), (1000, 3, 0x1_0000, -1000)] {
            trun.extend(d.to_be_bytes());
            trun.extend(s.to_be_bytes());
            trun.extend(f.to_be_bytes());
            trun.extend(c.to_be_bytes());
        }
        let trun_box = bx(b"trun", &trun);
        let traf = bx(b"traf", &[tfhd, tfdt, trun_box].concat());
        let moof_len = 8 + traf.len();
        let mut moof = bx(b"moof", &traf);
        // Data starts right after the moof's header + the mdat header.
        let data_off = (moof_len + 8) as u32;
        let pos = moof.windows(4).position(|w| w == b"trun").unwrap() + 4 + 8;
        moof[pos..pos + 4].copy_from_slice(&data_off.to_be_bytes());
        let mdat = bx(b"mdat", &[1, 2, 3, 4, 5, 6, 7, 8]);
        let data = [moof, mdat].concat();
        let t = Track { id: 1, timescale: 1000, codec: Codec::Other(*b"test"), width: 0, height: 0, delay: 1000, duration: 0, samples: vec![], default_duration: 0, default_size: 0, default_flags: 0 };
        let s = parse_fragment(&data, &t);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].offset, s[0].size, s[0].key), (data_off as usize, 5, true));
        assert_eq!((s[1].offset, s[1].size, s[1].key), (data_off as usize + 5, 3, false));
        assert_eq!(t.pts(&s[0]), 4.0);
        assert_eq!(t.pts(&s[1]), 2.0);
    }
}

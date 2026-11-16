//! macOS decoding and sound, with what macOS has built in: VideoToolbox for
//! H.264, AudioToolbox's converter for AAC, and the default output audio
//! unit for sound. Same shapes as `mf` and `wasapi` on Windows, so the
//! decoding threads (engine.rs) don't care which system they run on.

#![allow(non_upper_case_globals, non_snake_case, clippy::upper_case_acronyms)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;

type OSStatus = i32;
type CFRef = *const c_void;

fn check(what: &str, status: OSStatus) -> Result<()> {
    if status == 0 { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

// ------------------------------------------------------------ frameworks

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

impl CMTime {
    /// Microsecond precision is plenty for frame times.
    fn seconds(s: f64) -> Self {
        Self { value: (s * 1_000_000.0).round() as i64, timescale: 1_000_000, flags: 1, epoch: 0 }
    }
    fn invalid() -> Self {
        Self::default()
    }
    fn as_seconds(&self) -> Option<f64> {
        (self.flags & 1 != 0 && self.timescale != 0).then(|| self.value as f64 / self.timescale as f64)
    }
}

#[repr(C)]
struct CMSampleTimingInfo {
    duration: CMTime,
    pts: CMTime,
    dts: CMTime,
}

#[repr(C)]
struct VTOutputCallback {
    callback: extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CFRef, CMTime, CMTime),
    refcon: *mut c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFRelease(cf: CFRef);
    fn CFNumberCreate(allocator: CFRef, kind: isize, value: *const c_void) -> CFRef;
    fn CFDictionaryCreate(allocator: CFRef, keys: *const CFRef, values: *const CFRef, count: isize, key_cb: *const c_void, value_cb: *const c_void) -> CFRef;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFRef;
    fn CVPixelBufferLockBaseAddress(pb: CFRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: CFRef, flags: u64) -> i32;
    fn CVPixelBufferGetPlaneCount(pb: CFRef) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(pb: CFRef, plane: usize) -> *const u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(pb: CFRef, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pb: CFRef, plane: usize) -> usize;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFRef,
        count: usize,
        sets: *const *const u8,
        sizes: *const usize,
        nal_header_len: i32,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: CFRef,
        memory: *mut c_void,
        len: usize,
        block_allocator: CFRef,
        source: *const c_void,
        offset: usize,
        data_len: usize,
        flags: u32,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(source: *const c_void, dest: CFRef, offset: usize, len: usize) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFRef,
        data: CFRef,
        format: CFRef,
        samples: isize,
        timing_count: isize,
        timing: *const CMSampleTimingInfo,
        size_count: isize,
        sizes: *const usize,
        out: *mut CFRef,
    ) -> OSStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(allocator: CFRef, format: CFRef, decoder_spec: CFRef, image_attrs: CFRef, callback: *const VTOutputCallback, out: *mut CFRef) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(session: CFRef, sample: CFRef, flags: u32, frame_refcon: *mut c_void, info_out: *mut u32) -> OSStatus;
    fn VTDecompressionSessionFinishDelayedFrames(session: CFRef) -> OSStatus;
    fn VTDecompressionSessionWaitForAsynchronousFrames(session: CFRef) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: CFRef);
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ASBD {
    sample_rate: f64,
    format: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

impl ASBD {
    /// Interleaved 32-bit float samples.
    fn float(rate: f64, channels: u32) -> Self {
        Self {
            sample_rate: rate,
            format: fourcc(b"lpcm"),
            flags: 1 | 8, // float, packed
            bytes_per_packet: 4 * channels,
            frames_per_packet: 1,
            bytes_per_frame: 4 * channels,
            channels,
            bits: 32,
            reserved: 0,
        }
    }
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PacketDescription {
    start: i64,
    variable_frames: u32,
    size: u32,
}

#[repr(C)]
struct ComponentDescription {
    kind: u32,
    sub_kind: u32,
    manufacturer: u32,
    flags: u32,
    mask: u32,
}

#[repr(C)]
struct RenderCallback {
    proc_: extern "C" fn(*mut c_void, *mut u32, *const c_void, u32, u32, *mut AudioBufferList) -> OSStatus,
    refcon: *mut c_void,
}

type InputProc = extern "C" fn(CFRef, *mut u32, *mut AudioBufferList, *mut *mut PacketDescription, *mut c_void) -> OSStatus;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioFormatGetProperty(id: u32, spec_size: u32, spec: *const c_void, size: *mut u32, out: *mut c_void) -> OSStatus;
    fn AudioConverterNew(from: *const ASBD, to: *const ASBD, out: *mut CFRef) -> OSStatus;
    fn AudioConverterSetProperty(conv: CFRef, id: u32, size: u32, data: *const c_void) -> OSStatus;
    fn AudioConverterFillComplexBuffer(conv: CFRef, input: InputProc, user: *mut c_void, packets: *mut u32, out: *mut AudioBufferList, descs: *mut PacketDescription) -> OSStatus;
    fn AudioConverterReset(conv: CFRef) -> OSStatus;
    fn AudioConverterDispose(conv: CFRef) -> OSStatus;

    fn AudioComponentFindNext(after: CFRef, desc: *const ComponentDescription) -> CFRef;
    fn AudioComponentInstanceNew(component: CFRef, out: *mut CFRef) -> OSStatus;
    fn AudioComponentInstanceDispose(unit: CFRef) -> OSStatus;
    fn AudioUnitSetProperty(unit: CFRef, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> OSStatus;
    fn AudioUnitInitialize(unit: CFRef) -> OSStatus;
    fn AudioUnitUninitialize(unit: CFRef) -> OSStatus;
    fn AudioOutputUnitStart(unit: CFRef) -> OSStatus;
    fn AudioOutputUnitStop(unit: CFRef) -> OSStatus;
}

/// Nothing to set up per thread (Media Foundation needs COM on Windows).
pub fn init_thread() -> Result<()> {
    Ok(())
}

// ------------------------------------------------------------------ H.264

/// A decoded picture in NV12.
pub struct Picture {
    pub pts: f64,
    pub data: Vec<u8>,
    pub stride: usize,
    /// Rows in the luma plane.
    pub rows: usize,
}

/// Pictures come out of VideoToolbox in decoding order; this many are held
/// back and handed out by time, which puts B-frames in their place.
const REORDER: usize = 4;

pub struct H264 {
    session: CFRef,
    format: CFRef,
    params: (Vec<u8>, Vec<u8>),
    /// Filled by the output callback (it runs inside DecodeFrame).
    decoded: Box<Mutex<Vec<Picture>>>,
    held: Vec<Picture>,
    avcc: Vec<u8>,
}

// The session is created and used on the decoding thread only.
unsafe impl Send for H264 {}

extern "C" fn on_picture(refcon: *mut c_void, _frame: *mut c_void, status: OSStatus, _flags: u32, image: CFRef, pts: CMTime, _dur: CMTime) {
    if status != 0 || image.is_null() {
        return;
    }
    let out = unsafe { &*(refcon as *const Mutex<Vec<Picture>>) };
    if let Some(p) = unsafe { copy_nv12(image, pts.as_seconds().unwrap_or(0.0)) } {
        out.lock().unwrap_or_else(|e| e.into_inner()).push(p);
    }
}

/// Copy a bi-planar 4:2:0 pixel buffer into one NV12 block (both planes
/// with the luma plane's stride).
unsafe fn copy_nv12(pb: CFRef, pts: f64) -> Option<Picture> {
    unsafe {
        if CVPixelBufferLockBaseAddress(pb, 1) != 0 {
            return None;
        }
        let picture = (|| {
            if CVPixelBufferGetPlaneCount(pb) < 2 {
                return None;
            }
            let (y, uv) = (CVPixelBufferGetBaseAddressOfPlane(pb, 0), CVPixelBufferGetBaseAddressOfPlane(pb, 1));
            if y.is_null() || uv.is_null() {
                return None;
            }
            let (stride, rows) = (CVPixelBufferGetBytesPerRowOfPlane(pb, 0), CVPixelBufferGetHeightOfPlane(pb, 0));
            let (uv_stride, uv_rows) = (CVPixelBufferGetBytesPerRowOfPlane(pb, 1), CVPixelBufferGetHeightOfPlane(pb, 1));
            let mut data = Vec::with_capacity(stride * (rows + uv_rows));
            data.extend_from_slice(std::slice::from_raw_parts(y, stride * rows));
            for r in 0..uv_rows {
                let row = std::slice::from_raw_parts(uv.add(r * uv_stride), uv_stride.min(stride));
                data.extend_from_slice(row);
                data.resize(data.len() + stride - row.len(), 128);
            }
            Some(Picture { pts, data, stride, rows })
        })();
        CVPixelBufferUnlockBaseAddress(pb, 1);
        picture
    }
}

/// The NAL units of an Annex B access unit.
fn nal_units(au: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= au.len() {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let ends: Vec<usize> = starts.iter().skip(1).map(|&s| s - 3).chain(std::iter::once(au.len())).collect();
    starts.into_iter().zip(ends).map(move |(s, e)| {
        // A 4-byte start code leaves a zero at the end of the previous unit.
        let mut e = e;
        while e > s && au[e - 1] == 0 {
            e -= 1;
        }
        &au[s..e]
    })
}

impl H264 {
    /// The decoder starts with the first key frame (it brings SPS and PPS).
    pub fn new(_width: u32, _height: u32) -> Result<Self> {
        Ok(Self {
            session: std::ptr::null(),
            format: std::ptr::null(),
            params: (Vec::new(), Vec::new()),
            decoded: Box::new(Mutex::new(Vec::new())),
            held: Vec::new(),
            avcc: Vec::new(),
        })
    }

    fn close(&mut self) {
        unsafe {
            if !self.session.is_null() {
                VTDecompressionSessionWaitForAsynchronousFrames(self.session);
                VTDecompressionSessionInvalidate(self.session);
                CFRelease(self.session);
            }
            if !self.format.is_null() {
                CFRelease(self.format);
            }
        }
        self.session = std::ptr::null();
        self.format = std::ptr::null();
    }

    fn open(&mut self, sps: &[u8], pps: &[u8]) -> Result<()> {
        self.close();
        unsafe {
            let sets = [sps.as_ptr(), pps.as_ptr()];
            let sizes = [sps.len(), pps.len()];
            check("H.264 format", CMVideoFormatDescriptionCreateFromH264ParameterSets(std::ptr::null(), 2, sets.as_ptr(), sizes.as_ptr(), 4, &mut self.format))?;
            // Ask for NV12 (video range), which yuv.rs converts.
            let nv12 = fourcc(b"420v") as i32;
            let number = CFNumberCreate(std::ptr::null(), 3, (&nv12 as *const i32).cast());
            let keys = [kCVPixelBufferPixelFormatTypeKey];
            let values = [number];
            let attrs = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                (&raw const kCFTypeDictionaryValueCallBacks).cast(),
            );
            CFRelease(number);
            let cb = VTOutputCallback { callback: on_picture, refcon: (&*self.decoded as *const Mutex<Vec<Picture>>).cast_mut().cast() };
            let r = VTDecompressionSessionCreate(std::ptr::null(), self.format, std::ptr::null(), attrs, &cb, &mut self.session);
            CFRelease(attrs);
            check("H.264 decoder", r)?;
        }
        self.params = (sps.to_vec(), pps.to_vec());
        Ok(())
    }

    /// Feed one access unit (Annex B); decoded pictures go to `out`.
    pub fn decode(&mut self, au: &[u8], pts: f64, dur: f64, out: &mut Vec<Picture>) -> Result<()> {
        let (mut sps, mut pps) = (None, None);
        self.avcc.clear();
        for nal in nal_units(au) {
            match nal.first().map(|b| b & 0x1f) {
                Some(7) => sps = Some(nal),
                Some(8) => pps = Some(nal),
                Some(9) | None => {}
                Some(_) => {
                    self.avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    self.avcc.extend_from_slice(nal);
                }
            }
        }
        if let (Some(s), Some(p)) = (sps, pps) {
            if self.session.is_null() || self.params.0 != s || self.params.1 != p {
                self.drain(out)?;
                self.open(s, p)?;
            }
        }
        if self.session.is_null() || self.avcc.is_empty() {
            // Nothing to decode until a key frame arrives.
            return Ok(());
        }
        unsafe {
            let mut block: CFRef = std::ptr::null();
            check("block buffer", CMBlockBufferCreateWithMemoryBlock(std::ptr::null(), std::ptr::null_mut(), self.avcc.len(), std::ptr::null(), std::ptr::null(), 0, self.avcc.len(), 0, &mut block))?;
            let r = CMBlockBufferReplaceDataBytes(self.avcc.as_ptr().cast(), block, 0, self.avcc.len());
            if r != 0 {
                CFRelease(block);
                return check("block buffer", r);
            }
            let timing = CMSampleTimingInfo { duration: CMTime::seconds(dur), pts: CMTime::seconds(pts), dts: CMTime::invalid() };
            let size = self.avcc.len();
            let mut sample: CFRef = std::ptr::null();
            let r = CMSampleBufferCreateReady(std::ptr::null(), block, self.format, 1, 1, &timing, 1, &size, &mut sample);
            CFRelease(block);
            check("sample buffer", r)?;
            let r = VTDecompressionSessionDecodeFrame(self.session, sample, 0, std::ptr::null_mut(), std::ptr::null_mut());
            CFRelease(sample);
            check("decode", r)?;
        }
        self.collect(out, REORDER);
        Ok(())
    }

    /// Move decoded pictures out in time order, keeping `keep` back.
    fn collect(&mut self, out: &mut Vec<Picture>, keep: usize) {
        self.held.append(&mut self.decoded.lock().unwrap_or_else(|e| e.into_inner()));
        self.held.sort_by(|a, b| a.pts.total_cmp(&b.pts));
        let n = self.held.len().saturating_sub(keep);
        out.extend(self.held.drain(..n));
    }

    /// Get the pictures still inside the decoder (end of stream).
    pub fn drain(&mut self, out: &mut Vec<Picture>) -> Result<()> {
        if !self.session.is_null() {
            unsafe {
                VTDecompressionSessionFinishDelayedFrames(self.session);
                VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            }
        }
        self.collect(out, 0);
        Ok(())
    }

    /// Forget everything in flight (seeking).
    pub fn flush(&mut self) {
        if !self.session.is_null() {
            unsafe { VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
        }
        self.decoded.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.held.clear();
    }
}

impl Drop for H264 {
    fn drop(&mut self) {
        self.close();
    }
}

// -------------------------------------------------------------------- AAC

/// The AudioSpecificConfig wrapped in an MPEG-4 ES descriptor: the "magic
/// cookie" AudioToolbox wants for AAC (built the way FFmpeg does).
fn esds_cookie(config: &[u8]) -> Vec<u8> {
    fn descr(v: &mut Vec<u8>, tag: u8, size: usize) {
        v.push(tag);
        for i in (1..=3).rev() {
            v.push(((size >> (7 * i)) as u8) | 0x80);
        }
        v.push((size & 0x7f) as u8);
    }
    let mut v = Vec::new();
    descr(&mut v, 0x03, 3 + 5 + 13 + 5 + config.len());
    v.extend_from_slice(&[0, 0, 0]); // ES id, flags
    descr(&mut v, 0x04, 13 + 5 + config.len());
    v.push(0x40); // MPEG-4 audio
    v.push(0x15); // audio stream
    v.extend_from_slice(&[0; 3 + 4 + 4]); // buffer size, max and average bitrate
    descr(&mut v, 0x05, config.len());
    v.extend_from_slice(config);
    v
}

/// One compressed frame handed to the converter's input callback.
struct Feed {
    data: Vec<u8>,
    desc: PacketDescription,
    given: bool,
    channels: u32,
}

/// Returned by the input callback when it has no more data for this call.
const NO_MORE: OSStatus = 0x6e6f_6d72; // 'nomr'

extern "C" fn feed(_conv: CFRef, packets: *mut u32, list: *mut AudioBufferList, descs: *mut *mut PacketDescription, user: *mut c_void) -> OSStatus {
    let f = unsafe { &mut *(user as *mut Feed) };
    unsafe {
        if f.given || f.data.is_empty() {
            *packets = 0;
            return NO_MORE;
        }
        f.given = true;
        *packets = 1;
        let b = &mut (*list).buffers[0];
        b.data = f.data.as_mut_ptr().cast();
        b.size = f.data.len() as u32;
        b.channels = f.channels;
        f.desc = PacketDescription { start: 0, variable_frames: 0, size: f.data.len() as u32 };
        if !descs.is_null() {
            *descs = &mut f.desc;
        }
    }
    0
}

pub struct Aac {
    conv: CFRef,
    feed: Box<Feed>,
    pcm: Vec<f32>,
    pub rate: u32,
    pub channels: u16,
}

unsafe impl Send for Aac {}

impl Aac {
    /// `config` is the AudioSpecificConfig from the MP4's esds box.
    pub fn new(config: &[u8], rate: u32, channels: u16) -> Result<Self> {
        let cookie = esds_cookie(config);
        // Let AudioToolbox read the real format (HE-AAC, channel count) from the config.
        let mut from = ASBD { sample_rate: rate as f64, format: fourcc(b"aac "), frames_per_packet: 1024, channels: channels as u32, ..Default::default() };
        let mut size = std::mem::size_of::<ASBD>() as u32;
        let mut info = from;
        if unsafe { AudioFormatGetProperty(fourcc(b"fmti"), cookie.len() as u32, cookie.as_ptr().cast(), &mut size, (&mut info as *mut ASBD).cast()) } == 0 && info.sample_rate > 0.0 && info.channels > 0 {
            from = info;
        }
        let to = ASBD::float(from.sample_rate, from.channels);
        let mut conv: CFRef = std::ptr::null();
        check("AAC decoder", unsafe { AudioConverterNew(&from, &to, &mut conv) })?;
        let r = unsafe { AudioConverterSetProperty(conv, fourcc(b"dmgc"), cookie.len() as u32, cookie.as_ptr().cast()) };
        if r != 0 {
            unsafe { AudioConverterDispose(conv) };
            return check("AAC config", r);
        }
        Ok(Self {
            conv,
            feed: Box::new(Feed { data: Vec::new(), desc: PacketDescription::default(), given: false, channels: from.channels }),
            pcm: Vec::new(),
            rate: from.sample_rate as u32,
            channels: from.channels as u16,
        })
    }

    /// Decode one AAC frame; interleaved samples go to `out` as (pts, samples).
    pub fn decode(&mut self, frame: &[u8], pts: f64, _dur: f64, out: &mut Vec<(f64, Vec<f32>)>) -> Result<()> {
        self.feed.data.clear();
        self.feed.data.extend_from_slice(frame);
        self.feed.given = false;
        let ch = self.channels.max(1) as usize;
        // HE-AAC frames decode to 2048 samples per channel.
        let cap = 4096;
        self.pcm.resize(cap * ch, 0.0);
        let mut list = AudioBufferList { count: 1, buffers: [AudioBuffer { channels: ch as u32, size: (self.pcm.len() * 4) as u32, data: self.pcm.as_mut_ptr().cast() }] };
        let mut frames = cap as u32;
        let r = unsafe { AudioConverterFillComplexBuffer(self.conv, feed, (&mut *self.feed as *mut Feed).cast(), &mut frames, &mut list, std::ptr::null_mut()) };
        if r != 0 && r != NO_MORE {
            return check("AAC decode", r);
        }
        if frames > 0 {
            out.push((pts, self.pcm[..frames as usize * ch].to_vec()));
        }
        Ok(())
    }

    pub fn flush(&mut self) {
        unsafe { AudioConverterReset(self.conv) };
    }

    pub fn drain(&mut self, _out: &mut Vec<(f64, Vec<f32>)>) -> Result<()> {
        Ok(())
    }
}

impl Drop for Aac {
    fn drop(&mut self) {
        unsafe { AudioConverterDispose(self.conv) };
    }
}

// ------------------------------------------------------------------ sound

/// What the output unit's render callback reads from.
struct Ring {
    samples: VecDeque<f32>,
    /// Frames handed to the device since the last reset.
    played: u64,
    volume: f32,
}

struct RingShared {
    ring: Mutex<Ring>,
    /// Signalled when the device took data (room for more).
    cv: Condvar,
    channels: usize,
}

extern "C" fn render(refcon: *mut c_void, _flags: *mut u32, _time: *const c_void, _bus: u32, frames: u32, list: *mut AudioBufferList) -> OSStatus {
    let sh = unsafe { &*(refcon as *const RingShared) };
    let b = unsafe { &mut (*list).buffers[0] };
    let out = unsafe { std::slice::from_raw_parts_mut(b.data as *mut f32, b.size as usize / 4) };
    let want = (frames as usize * sh.channels).min(out.len());
    let mut r = sh.ring.lock().unwrap_or_else(|e| e.into_inner());
    let n = (r.samples.len().min(want) / sh.channels) * sh.channels;
    let vol = r.volume;
    for (o, s) in out.iter_mut().zip(r.samples.drain(..n)) {
        *o = s * vol;
    }
    out[n..].fill(0.0);
    r.played += (n / sh.channels) as u64;
    drop(r);
    sh.cv.notify_all();
    0
}

pub struct Output {
    unit: CFRef,
    sh: Arc<RingShared>,
    rate: u32,
    /// Frames the queue holds at most (like WASAPI's buffer).
    buffer: u32,
    pub channels: u16,
}

unsafe impl Send for Output {}

impl Output {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        let sh = Arc::new(RingShared { ring: Mutex::new(Ring { samples: VecDeque::new(), played: 0, volume: 1.0 }), cv: Condvar::new(), channels: channels.max(1) as usize });
        unsafe {
            let desc = ComponentDescription { kind: fourcc(b"auou"), sub_kind: fourcc(b"def "), manufacturer: fourcc(b"appl"), flags: 0, mask: 0 };
            let comp = AudioComponentFindNext(std::ptr::null(), &desc);
            if comp.is_null() {
                return Err("no audio output".into());
            }
            let mut unit: CFRef = std::ptr::null();
            check("audio output", AudioComponentInstanceNew(comp, &mut unit))?;
            let out = Self { unit, sh: sh.clone(), rate, buffer: rate / 5, channels };
            // macOS converts our rate and layout to the device's.
            let fmt = ASBD::float(rate as f64, channels as u32);
            check("audio format", AudioUnitSetProperty(unit, 8, 1, 0, (&fmt as *const ASBD).cast(), std::mem::size_of::<ASBD>() as u32))?;
            let cb = RenderCallback { proc_: render, refcon: Arc::as_ptr(&sh).cast_mut().cast() };
            check("audio callback", AudioUnitSetProperty(unit, 23, 1, 0, (&cb as *const RenderCallback).cast(), std::mem::size_of::<RenderCallback>() as u32))?;
            check("audio output", AudioUnitInitialize(unit))?;
            Ok(out)
        }
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.sh.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Frames that can be written now.
    pub fn free(&self) -> Result<u32> {
        Ok(self.buffer.saturating_sub(self.queued()?))
    }

    /// Frames written but not played yet.
    pub fn queued(&self) -> Result<u32> {
        Ok((self.ring().samples.len() / self.sh.channels) as u32)
    }

    /// Write interleaved samples (a whole number of frames).
    pub fn write(&self, samples: &[f32]) -> Result<()> {
        self.ring().samples.extend(samples);
        Ok(())
    }

    /// Seconds played since the last reset.
    pub fn played(&self) -> Result<f64> {
        Ok(self.ring().played as f64 / self.rate.max(1) as f64)
    }

    pub fn start(&self) -> Result<()> {
        check("audio start", unsafe { AudioOutputUnitStart(self.unit) })
    }

    pub fn stop(&self) -> Result<()> {
        check("audio stop", unsafe { AudioOutputUnitStop(self.unit) })
    }

    /// Drop everything queued and restart the clock at zero (stopped first).
    pub fn reset(&self) -> Result<()> {
        let _ = self.stop();
        let mut r = self.ring();
        r.samples.clear();
        r.played = 0;
        Ok(())
    }

    pub fn set_volume(&self, v: f32) -> Result<()> {
        self.ring().volume = v.clamp(0.0, 1.0);
        Ok(())
    }

    /// Wait until the device wants more data (or the timeout passes).
    pub fn wait(&self, ms: u32) {
        let r = self.ring();
        let _ = self.sh.cv.wait_timeout(r, Duration::from_millis(ms as u64));
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_annex_b() {
        let au = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 5];
        let units: Vec<&[u8]> = nal_units(&au).collect();
        assert_eq!(units, [&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4, 5]]);
    }

    #[test]
    fn cookie_wraps_config() {
        let c = esds_cookie(&[0x12, 0x10]);
        assert_eq!(c[0], 0x03);
        assert_eq!(&c[c.len() - 2..], &[0x12, 0x10]);
        assert_eq!(c.len(), 5 + 3 + 5 + 13 + 5 + 2);
    }

    /// The decoders and the output device open (no files needed).
    #[test]
    fn opens_aac_and_output() {
        // AAC-LC, 44.1 kHz, stereo.
        let aac = Aac::new(&[0x12, 0x10], 44_100, 2).expect("AAC decoder");
        assert_eq!((aac.rate, aac.channels), (44_100, 2));
        // CI machines may have no sound device; just don't crash.
        let _ = Output::new(44_100, 2);
    }
}

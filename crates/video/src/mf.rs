//! Decoding with Windows Media Foundation's built-in H.264 and AAC decoders
//! (shipped with Windows, so no codecs to bundle). One decoder per thread.

use std::mem::ManuallyDrop;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::core::{GUID, Result};

/// Initialize COM + Media Foundation for the calling thread.
pub fn init_thread() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)
    }
}

const HNS: f64 = 10_000_000.0;

fn sample(data: &[u8], pts: f64, dur: f64) -> Result<IMFSample> {
    unsafe {
        let buf = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut p = std::ptr::null_mut();
        buf.Lock(&mut p, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        buf.Unlock()?;
        buf.SetCurrentLength(data.len() as u32)?;
        let s = MFCreateSample()?;
        s.AddBuffer(&buf)?;
        s.SetSampleTime((pts * HNS) as i64)?;
        s.SetSampleDuration((dur * HNS) as i64)?;
        Ok(s)
    }
}

/// Shared push/pull logic for a single-stream transform.
struct Mft {
    t: IMFTransform,
    out_size: u32,
    provides: bool,
}

enum Pulled {
    Sample(IMFSample),
    NeedInput,
    Changed,
}

impl Mft {
    fn new(clsid: &GUID) -> Result<Self> {
        let t: IMFTransform = unsafe { CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER)? };
        Ok(Self { t, out_size: 0, provides: false })
    }

    fn start(&mut self) -> Result<()> {
        unsafe {
            self.refresh_info()?;
            self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
        }
    }

    unsafe fn refresh_info(&mut self) -> Result<()> {
        let info = unsafe { self.t.GetOutputStreamInfo(0)? };
        self.provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
        self.out_size = info.cbSize.max(4096);
        Ok(())
    }

    /// Pick the first output type with this subtype.
    fn set_output(&mut self, subtype: &GUID) -> Result<IMFMediaType> {
        unsafe {
            let mut i = 0;
            loop {
                let mt = self.t.GetOutputAvailableType(0, i)?;
                if mt.GetGUID(&MF_MT_SUBTYPE)? == *subtype {
                    self.t.SetOutputType(0, &mt, 0)?;
                    self.refresh_info()?;
                    return Ok(mt);
                }
                i += 1;
            }
        }
    }

    /// Feed one sample. Returns false if the decoder wants output pulled first.
    fn push(&mut self, s: &IMFSample) -> Result<bool> {
        match unsafe { self.t.ProcessInput(0, s, 0) } {
            Ok(()) => Ok(true),
            Err(e) if e.code() == MF_E_NOTACCEPTING => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn pull(&mut self) -> Result<Pulled> {
        unsafe {
            let given = if self.provides {
                None
            } else {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreateMemoryBuffer(self.out_size)?)?;
                Some(s)
            };
            let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(given), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let r = self.t.ProcessOutput(0, &mut out, &mut status);
            let sample = ManuallyDrop::take(&mut out[0].pSample);
            drop(ManuallyDrop::take(&mut out[0].pEvents));
            match r {
                Ok(()) => Ok(sample.map(Pulled::Sample).unwrap_or(Pulled::NeedInput)),
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(Pulled::NeedInput),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => Ok(Pulled::Changed),
                Err(e) => Err(e),
            }
        }
    }

    fn flush(&mut self) {
        unsafe {
            let _ = self.t.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
    }

    fn drain(&mut self) {
        unsafe {
            let _ = self.t.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);
        }
    }
}

fn with_bytes<R>(s: &IMFSample, f: impl FnOnce(&[u8]) -> R) -> Result<R> {
    unsafe {
        let buf = s.ConvertToContiguousBuffer()?;
        let mut p = std::ptr::null_mut();
        let mut len = 0u32;
        buf.Lock(&mut p, None, Some(&mut len))?;
        let r = f(std::slice::from_raw_parts(p, len as usize));
        buf.Unlock()?;
        Ok(r)
    }
}

/// A decoded picture in NV12.
pub struct Picture {
    pub pts: f64,
    pub data: Vec<u8>,
    pub stride: usize,
    /// Rows in the luma plane (may be padded past the visible height).
    pub rows: usize,
}

pub struct H264 {
    m: Mft,
    stride: usize,
    rows: usize,
}

impl H264 {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let m = Mft::new(&CLSID_MSH264DecoderMFT)?;
        unsafe {
            let it = MFCreateMediaType()?;
            it.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            it.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            if width > 0 && height > 0 {
                it.SetUINT64(&MF_MT_FRAME_SIZE, (width as u64) << 32 | height as u64)?;
            }
            it.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            m.t.SetInputType(0, &it, 0)?;
        }
        let mut d = Self { m, stride: width as usize, rows: height as usize };
        d.configure()?;
        d.m.start()?;
        Ok(d)
    }

    fn configure(&mut self) -> Result<()> {
        let mt = self.m.set_output(&MFVideoFormat_NV12)?;
        unsafe {
            if let Ok(size) = mt.GetUINT64(&MF_MT_FRAME_SIZE) {
                self.stride = (size >> 32) as usize;
                self.rows = (size & 0xffff_ffff) as usize;
            }
            if let Ok(s) = mt.GetUINT32(&MF_MT_DEFAULT_STRIDE) {
                let s = s as i32;
                if s > 0 {
                    self.stride = s as usize;
                }
            }
        }
        Ok(())
    }

    /// Feed one access unit (Annex B); decoded pictures go to `out`.
    pub fn decode(&mut self, au: &[u8], pts: f64, dur: f64, out: &mut Vec<Picture>) -> Result<()> {
        let s = sample(au, pts, dur)?;
        loop {
            if self.m.push(&s)? {
                break;
            }
            self.pull_all(out)?;
        }
        self.pull_all(out)
    }

    /// Get the pictures still inside the decoder (end of stream).
    pub fn drain(&mut self, out: &mut Vec<Picture>) -> Result<()> {
        self.m.drain();
        self.pull_all(out)?;
        // Ready for more input after a drain.
        unsafe {
            let _ = self.m.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
        }
        Ok(())
    }

    pub fn flush(&mut self) {
        self.m.flush();
    }

    fn pull_all(&mut self, out: &mut Vec<Picture>) -> Result<()> {
        loop {
            match self.m.pull()? {
                Pulled::NeedInput => return Ok(()),
                Pulled::Changed => self.configure()?,
                Pulled::Sample(s) => {
                    let pts = unsafe { s.GetSampleTime().unwrap_or(0) } as f64 / HNS;
                    let (stride, rows) = (self.stride, self.rows);
                    let data = with_bytes(&s, |b| b.to_vec())?;
                    // A buffer smaller than stride*rows*1.5 means the rows aren't padded.
                    let rows = if data.len() >= stride * rows * 3 / 2 { rows } else { data.len() * 2 / 3 / stride.max(1) };
                    out.push(Picture { pts, data, stride, rows });
                }
            }
        }
    }
}

pub struct Aac {
    m: Mft,
    pub rate: u32,
    pub channels: u16,
    float: bool,
}

impl Aac {
    /// `config` is the AudioSpecificConfig from the MP4's esds box.
    pub fn new(config: &[u8], rate: u32, channels: u16) -> Result<Self> {
        let m = Mft::new(&CLSID_MSAACDecMFT)?;
        unsafe {
            let it = MFCreateMediaType()?;
            it.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            it.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            it.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            it.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
            it.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            it.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
            // HEAACWAVEINFO after its WAVEFORMATEX: payload type 0 (raw),
            // profile 0xFE (unknown), then the AudioSpecificConfig.
            let mut user = vec![0u8; 12];
            user[2] = 0xfe;
            user.extend_from_slice(config);
            it.SetBlob(&MF_MT_USER_DATA, &user)?;
            m.t.SetInputType(0, &it, 0)?;
        }
        let mut a = Self { m, rate, channels, float: true };
        a.configure()?;
        a.m.start()?;
        Ok(a)
    }

    fn configure(&mut self) -> Result<()> {
        let mt = match self.m.set_output(&MFAudioFormat_Float) {
            Ok(mt) => {
                self.float = true;
                mt
            }
            Err(_) => {
                self.float = false;
                self.m.set_output(&MFAudioFormat_PCM)?
            }
        };
        unsafe {
            self.rate = mt.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(self.rate);
            self.channels = mt.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).map(|c| c as u16).unwrap_or(self.channels);
        }
        Ok(())
    }

    /// Decode one AAC frame; interleaved samples go to `out` as (pts, samples).
    pub fn decode(&mut self, frame: &[u8], pts: f64, dur: f64, out: &mut Vec<(f64, Vec<f32>)>) -> Result<()> {
        let s = sample(frame, pts, dur)?;
        loop {
            if self.m.push(&s)? {
                break;
            }
            self.pull_all(out)?;
        }
        self.pull_all(out)
    }

    pub fn flush(&mut self) {
        self.m.flush();
    }

    pub fn drain(&mut self, out: &mut Vec<(f64, Vec<f32>)>) -> Result<()> {
        self.m.drain();
        self.pull_all(out)?;
        unsafe {
            let _ = self.m.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
        }
        Ok(())
    }

    fn pull_all(&mut self, out: &mut Vec<(f64, Vec<f32>)>) -> Result<()> {
        loop {
            match self.m.pull()? {
                Pulled::NeedInput => return Ok(()),
                Pulled::Changed => self.configure()?,
                Pulled::Sample(s) => {
                    let pts = unsafe { s.GetSampleTime().unwrap_or(0) } as f64 / HNS;
                    let float = self.float;
                    let pcm = with_bytes(&s, |b| {
                        if float {
                            b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect::<Vec<f32>>()
                        } else {
                            b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()
                        }
                    })?;
                    out.push((pts, pcm));
                }
            }
        }
    }
}

//! Audio output through WASAPI (shared mode). Windows converts our sample
//! rate and channel layout to the device's, and its clock tells how much has
//! actually been heard, which drives A/V sync.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::Result;

pub struct Output {
    client: IAudioClient,
    render: IAudioRenderClient,
    clock: IAudioClock,
    volume: IAudioStreamVolume,
    event: HANDLE,
    buffer: u32,
    freq: u64,

    pub channels: u16,
}

// The COM objects are created on, and only used from, the audio thread.

impl Output {
    pub fn new(rate: u32, channels: u16) -> Result<Self> {
        unsafe {
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let block = 4 * channels;
            let fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
                nChannels: channels,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * block as u32,
                nBlockAlign: block,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            // 100 ms buffer.
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, &fmt, None)?;
            let event = CreateEventW(None, false, false, None)?;
            client.SetEventHandle(event)?;
            let render: IAudioRenderClient = client.GetService()?;
            let clock: IAudioClock = client.GetService()?;
            let volume: IAudioStreamVolume = client.GetService()?;
            let buffer = client.GetBufferSize()?;
            let freq = clock.GetFrequency()?.max(1);
            Ok(Self { client, render, clock, volume, event, buffer, freq, channels })
        }
    }

    /// Frames that can be written now.
    pub fn free(&self) -> Result<u32> {
        Ok(self.buffer.saturating_sub(self.queued()?))
    }

    /// Frames written but not played yet.
    pub fn queued(&self) -> Result<u32> {
        unsafe { self.client.GetCurrentPadding() }
    }

    /// Write interleaved samples (a whole number of frames).
    pub fn write(&self, samples: &[f32]) -> Result<()> {
        let frames = (samples.len() / self.channels as usize) as u32;
        if frames == 0 {
            return Ok(());
        }
        unsafe {
            let p = self.render.GetBuffer(frames)? as *mut f32;
            std::ptr::copy_nonoverlapping(samples.as_ptr(), p, frames as usize * self.channels as usize);
            self.render.ReleaseBuffer(frames, 0)
        }
    }

    /// Seconds played since the last reset.
    pub fn played(&self) -> Result<f64> {
        let mut pos = 0u64;
        unsafe { self.clock.GetPosition(&mut pos, None)? };
        Ok(pos as f64 / self.freq as f64)
    }

    pub fn start(&self) -> Result<()> {
        unsafe { self.client.Start() }
    }

    pub fn stop(&self) -> Result<()> {
        unsafe { self.client.Stop() }
    }

    /// Drop everything queued and restart the clock at zero (stopped first).
    pub fn reset(&self) -> Result<()> {
        unsafe {
            let _ = self.client.Stop();
            self.client.Reset()
        }
    }

    pub fn set_volume(&self, v: f32) -> Result<()> {
        let all = vec![v.clamp(0.0, 1.0); self.channels as usize];
        unsafe { self.volume.SetAllVolumes(&all) }
    }

    /// Wait until the device wants more data (or the timeout passes).
    pub fn wait(&self, ms: u32) {
        unsafe {
            WaitForSingleObject(self.event, ms);
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

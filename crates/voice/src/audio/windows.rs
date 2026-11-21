//! Microphones and speakers through WASAPI (shared mode). Windows converts
//! our 48 kHz mono to and from whatever the device uses.

use super::{Device, Devices, OnFrame, Pull, Stream};
use crate::codec::{FRAME, SAMPLE_RATE};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{HSTRING, Interface, PCWSTR};

/// 60 ms: enough to ride out a busy moment without adding much delay.
const BUFFER_100NS: i64 = 600_000;

struct Com(bool);

impl Com {
    fn init() -> Self {
        // Already set up differently on this thread is fine.
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

fn text(e: windows::core::Error) -> String {
    e.message().to_string()
}

fn list(devices: &IMMDeviceEnumerator, flow: EDataFlow) -> windows::core::Result<Vec<Device>> {
    let mut out = Vec::new();
    unsafe {
        let all = devices.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..all.GetCount()? {
            let d = all.Item(i)?;
            let raw = d.GetId()?;
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const _));
            let store = d.OpenPropertyStore(STGM_READ)?;
            let mut value = store.GetValue(&PKEY_Device_FriendlyName)?;
            let name = value.to_string();
            let _ = PropVariantClear(&mut value);
            out.push(Device { name: if name.is_empty() { id.clone() } else { name }, id });
        }
    }
    Ok(out)
}

pub fn devices() -> Result<Devices, String> {
    let _com = Com::init();
    unsafe {
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(text)?;
        Ok(Devices { inputs: list(&e, eCapture).map_err(text)?, outputs: list(&e, eRender).map_err(text)? })
    }
}

/// The chosen device, or the system's default if it's gone.
fn pick(flow: EDataFlow, id: Option<&str>) -> windows::core::Result<IMMDevice> {
    unsafe {
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        if let Some(id) = id {
            let wide = HSTRING::from(id);
            if let Ok(d) = e.GetDevice(PCWSTR(wide.as_ptr())) {
                return Ok(d);
            }
            tracing::info!("voice: the chosen audio device is gone, using the default");
        }
        e.GetDefaultAudioEndpoint(flow, eConsole)
    }
}

fn format(channels: u16) -> WAVEFORMATEX {
    let block = 4 * channels;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: channels,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * block as u32,
        nBlockAlign: block,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// An audio client set up for this format, waking us through an event.
fn open_client(device: &IMMDevice, channels: u16, communications: bool) -> windows::core::Result<(IAudioClient, HANDLE)> {
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        if communications && let Ok(c2) = client.cast::<IAudioClient2>() {
            // Asks Windows for its voice processing (like echo removal) where
            // the device has it.
            let props = AudioClientProperties {
                cbSize: std::mem::size_of::<AudioClientProperties>() as u32,
                bIsOffload: false.into(),
                eCategory: AudioCategory_Communications,
                Options: AUDCLNT_STREAMOPTIONS_NONE,
            };
            let _ = c2.SetClientProperties(&props);
        }
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_100NS, 0, &format(channels), None)?;
        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        Ok((client, event))
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Stream for Running {}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Start a device thread and wait until it's opened (or failed).
fn spawn(name: &str, body: impl FnOnce(Arc<AtomicBool>, mpsc::Sender<Result<(), String>>) + Send + 'static) -> Result<Box<dyn Stream>, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let flag = stop.clone();
    let thread = std::thread::Builder::new().name(name.into()).spawn(move || body(flag, tx)).map_err(|e| e.to_string())?;
    let running = Running { stop, thread: Some(thread) };
    match rx.recv() {
        Ok(Ok(())) => Ok(Box::new(running)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the audio device stopped".into()),
    }
}

pub fn open_input(device: Option<&str>, mut on_frame: OnFrame) -> Result<Box<dyn Stream>, String> {
    let device = device.map(str::to_string);
    spawn("voice-mic", move |stop, ready| {
        let _com = Com::init();
        let opened = (|| -> windows::core::Result<_> {
            let d = pick(eCapture, device.as_deref())?;
            let (client, event) = open_client(&d, 1, true)?;
            let capture: IAudioCaptureClient = unsafe { client.GetService()? };
            unsafe { client.Start()? };
            Ok((client, event, capture))
        })();
        let (client, event, capture) = match opened {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(text(e)));
                return;
            }
        };
        let mut frame = Vec::with_capacity(FRAME);
        'run: while !stop.load(Ordering::Relaxed) {
            unsafe { WaitForSingleObject(event, 100) };
            loop {
                let waiting = match unsafe { capture.GetNextPacketSize() } {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::warn!("voice: microphone stopped: {e}");
                        break 'run;
                    }
                };
                if waiting == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if let Err(e) = unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) } {
                    tracing::warn!("voice: microphone stopped: {e}");
                    break 'run;
                }
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                for i in 0..frames as usize {
                    frame.push(if silent { 0.0 } else { unsafe { *(data as *const f32).add(i) } });
                    if frame.len() == FRAME {
                        on_frame(&frame);
                        frame.clear();
                    }
                }
                let _ = unsafe { capture.ReleaseBuffer(frames) };
            }
        }
        unsafe {
            let _ = client.Stop();
            let _ = CloseHandle(event);
        }
    })
}

pub fn open_output(device: Option<&str>, mut pull: Pull) -> Result<Box<dyn Stream>, String> {
    let device = device.map(str::to_string);
    spawn("voice-speakers", move |stop, ready| {
        let _com = Com::init();
        let opened = (|| -> windows::core::Result<_> {
            let d = pick(eRender, device.as_deref())?;
            let (client, event) = open_client(&d, 2, false)?;
            let render: IAudioRenderClient = unsafe { client.GetService()? };
            let size = unsafe { client.GetBufferSize()? };
            unsafe { client.Start()? };
            Ok((client, event, render, size))
        })();
        let (client, event, render, size) = match opened {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(text(e)));
                return;
            }
        };
        let mut mono = vec![0f32; FRAME];
        let mut left = 0usize; // samples of `mono` not written yet
        'run: while !stop.load(Ordering::Relaxed) {
            unsafe { WaitForSingleObject(event, 100) };
            let queued = match unsafe { client.GetCurrentPadding() } {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("voice: speakers stopped: {e}");
                    break 'run;
                }
            };
            let mut free = size.saturating_sub(queued) as usize;
            while free > 0 {
                if left == 0 {
                    pull(&mut mono);
                    left = FRAME;
                }
                let n = free.min(left);
                let from = FRAME - left;
                match unsafe { render.GetBuffer(n as u32) } {
                    Ok(p) => unsafe {
                        let out = std::slice::from_raw_parts_mut(p as *mut f32, n * 2);
                        for (pair, s) in out.chunks_exact_mut(2).zip(&mono[from..from + n]) {
                            pair[0] = *s;
                            pair[1] = *s;
                        }
                        let _ = render.ReleaseBuffer(n as u32, 0);
                    },
                    Err(e) => {
                        tracing::warn!("voice: speakers stopped: {e}");
                        break 'run;
                    }
                }
                left -= n;
                free -= n;
            }
        }
        unsafe {
            let _ = client.Stop();
            let _ = CloseHandle(event);
        }
    })
}

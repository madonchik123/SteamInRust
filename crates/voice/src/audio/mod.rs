//! Microphones and speakers. Everything here is 48 kHz, one channel, in
//! 20 ms frames; the system converts to and from what the device uses.

#[cfg(target_vendor = "apple")]
#[path = "macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(not(any(windows, target_vendor = "apple")))]
#[path = "none.rs"]
mod platform;

/// A microphone or speaker the system knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Stays the same between runs, so a choice can be saved.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
}

/// List what's plugged in.
pub fn devices() -> Devices {
    platform::devices().unwrap_or_else(|e| {
        tracing::warn!("voice: can't list audio devices: {e}");
        Devices::default()
    })
}

/// Gets each 20 ms from the microphone.
pub type OnFrame = Box<dyn FnMut(&[f32]) + Send>;
/// Fills the next 20 ms for the speakers.
pub type Pull = Box<dyn FnMut(&mut [f32]) + Send>;

/// Something playing or recording; dropping it stops it.
pub trait Stream: Send {}

/// Where audio comes from and goes to (tests use a stand-in).
pub trait Backend: Send {
    fn open_input(&self, device: Option<&str>, on_frame: OnFrame) -> Result<Box<dyn Stream>, String>;
    fn open_output(&self, device: Option<&str>, pull: Pull) -> Result<Box<dyn Stream>, String>;
}

/// The real devices.
pub struct System;

impl Backend for System {
    fn open_input(&self, device: Option<&str>, on_frame: OnFrame) -> Result<Box<dyn Stream>, String> {
        platform::open_input(device, on_frame)
    }

    fn open_output(&self, device: Option<&str>, pull: Pull) -> Result<Box<dyn Stream>, String> {
        platform::open_output(device, pull)
    }
}

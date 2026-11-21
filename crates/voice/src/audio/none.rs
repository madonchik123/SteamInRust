//! Systems without voice chat audio yet.

use super::{Devices, OnFrame, Pull, Stream};

pub fn devices() -> Result<Devices, String> {
    Ok(Devices::default())
}

pub fn open_input(_device: Option<&str>, _on_frame: OnFrame) -> Result<Box<dyn Stream>, String> {
    Err("Voice chat audio isn't available on this system".into())
}

pub fn open_output(_device: Option<&str>, _pull: Pull) -> Result<Box<dyn Stream>, String> {
    Err("Voice chat audio isn't available on this system".into())
}

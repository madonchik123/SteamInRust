//! Opus, the voice format Steam uses: 48 kHz, one channel, 20 ms packets.

use opusic_sys as sys;
use std::ffi::c_int;

pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in the 20 ms frames we work in.
pub const FRAME: usize = 960;
/// The longest a single Opus packet can decode to (120 ms).
pub const MAX_DECODED: usize = 5760;

fn describe(code: c_int) -> String {
    format!("Opus error {code}")
}

pub struct Encoder(*mut sys::OpusEncoder);

// The state is only ever used through `&mut self`.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Set up for speech the way Steam's client does: loss recovery on, and
    /// almost nothing sent during silence.
    pub fn new() -> Result<Self, String> {
        let mut err = 0;
        let st = unsafe { sys::opus_encoder_create(SAMPLE_RATE as i32, 1, sys::OPUS_APPLICATION_VOIP, &mut err) };
        if st.is_null() || err != sys::OPUS_OK {
            return Err(describe(err));
        }
        let e = Self(st);
        for (request, value) in [
            (sys::OPUS_SET_BITRATE_REQUEST, 32_000),
            (sys::OPUS_SET_SIGNAL_REQUEST, sys::OPUS_SIGNAL_VOICE),
            (sys::OPUS_SET_INBAND_FEC_REQUEST, 1),
            (sys::OPUS_SET_PACKET_LOSS_PERC_REQUEST, 5),
            (sys::OPUS_SET_DTX_REQUEST, 1),
        ] {
            let r = unsafe { sys::opus_encoder_ctl(e.0, request, value as c_int) };
            if r != sys::OPUS_OK {
                return Err(describe(r));
            }
        }
        Ok(e)
    }

    /// Encode one 20 ms frame; returns the packet's length.
    pub fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Result<usize, String> {
        assert_eq!(pcm.len(), FRAME);
        let n = unsafe { sys::opus_encode_float(self.0, pcm.as_ptr(), FRAME as c_int, out.as_mut_ptr(), out.len() as i32) };
        if n < 0 { Err(describe(n)) } else { Ok(n as usize) }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { sys::opus_encoder_destroy(self.0) }
    }
}

pub struct Decoder(*mut sys::OpusDecoder);

// The state is only ever used through `&mut self`.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Result<Self, String> {
        let mut err = 0;
        let st = unsafe { sys::opus_decoder_create(SAMPLE_RATE as i32, 1, &mut err) };
        if st.is_null() || err != sys::OPUS_OK {
            return Err(describe(err));
        }
        Ok(Self(st))
    }

    /// Decode a packet; returns how many samples were written.
    pub fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        let n = unsafe { sys::opus_decode_float(self.0, packet.as_ptr(), packet.len() as i32, out.as_mut_ptr(), out.len() as c_int, 0) };
        if n < 0 { Err(describe(n)) } else { Ok(n as usize) }
    }

    /// Rebuild a lost 20 ms frame from the recovery data in the packet after it.
    pub fn recover(&mut self, next_packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        let n = unsafe { sys::opus_decode_float(self.0, next_packet.as_ptr(), next_packet.len() as i32, out.as_mut_ptr(), FRAME as c_int, 1) };
        if n < 0 { Err(describe(n)) } else { Ok(n as usize) }
    }

    /// Make up 20 ms to hide a lost packet.
    pub fn conceal(&mut self, out: &mut [f32]) -> Result<usize, String> {
        let n = unsafe { sys::opus_decode_float(self.0, std::ptr::null(), 0, out.as_mut_ptr(), FRAME as c_int, 0) };
        if n < 0 { Err(describe(n)) } else { Ok(n as usize) }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { sys::opus_decoder_destroy(self.0) }
    }
}

#[cfg(test)]
pub(crate) fn tone(frame: usize, hz: f32) -> Vec<f32> {
    (0..FRAME).map(|i| (((frame * FRAME + i) as f32 / SAMPLE_RATE as f32) * hz * std::f32::consts::TAU).sin() * 0.3).collect()
}

/// The pitch of a signal from how often it crosses zero.
#[cfg(test)]
pub(crate) fn pitch(pcm: &[f32]) -> f32 {
    let crossings = pcm.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32;
    crossings / 2.0 / (pcm.len() as f32 / SAMPLE_RATE as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_survives_encoding() {
        let (mut e, mut d) = (Encoder::new().unwrap(), Decoder::new().unwrap());
        let mut heard = Vec::new();
        for f in 0..50 {
            let mut packet = [0u8; 400];
            let n = e.encode(&tone(f, 440.0), &mut packet).unwrap();
            let mut pcm = [0f32; FRAME];
            assert_eq!(d.decode(&packet[..n], &mut pcm).unwrap(), FRAME);
            heard.extend_from_slice(&pcm);
        }
        let hz = pitch(&heard[FRAME * 10..]);
        assert!((hz - 440.0).abs() < 8.0, "heard {hz} Hz");
    }

    #[test]
    fn silence_costs_almost_nothing() {
        let mut e = Encoder::new().unwrap();
        let mut sizes = Vec::new();
        for _ in 0..100 {
            let mut packet = [0u8; 400];
            sizes.push(e.encode(&[0.0; FRAME], &mut packet).unwrap());
        }
        // Almost every frame is the 1-byte "still silent" marker, with a small
        // refresh of the background noise now and then.
        let tiny = sizes[50..].iter().filter(|n| **n <= 2).count();
        assert!(tiny >= 40, "{sizes:?}");
        assert!(sizes[50..].iter().all(|n| *n < 60), "{sizes:?}");
    }

    #[test]
    fn a_lost_packet_is_covered() {
        let (mut e, mut d) = (Encoder::new().unwrap(), Decoder::new().unwrap());
        let mut last = Vec::new();
        for f in 0..30 {
            let mut packet = [0u8; 400];
            let n = e.encode(&tone(f, 440.0), &mut packet).unwrap();
            let mut pcm = [0f32; FRAME];
            d.decode(&packet[..n], &mut pcm).unwrap();
            last = packet[..n].to_vec();
        }
        let mut pcm = [0f32; FRAME];
        assert_eq!(d.conceal(&mut pcm).unwrap(), FRAME);
        assert_eq!(d.recover(&last, &mut pcm).unwrap(), FRAME);
    }
}

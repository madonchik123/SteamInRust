//! The ringing sound while a call waits to be picked up: two soft tones,
//! two seconds on, three off. Made here, so no sound files are needed.

use crate::audio::{Backend, System};
use crate::codec::{FRAME, SAMPLE_RATE};
use std::sync::mpsc;

const ON_SECONDS: f32 = 2.0;
const PERIOD_SECONDS: f32 = 5.0;
const VOLUME: f32 = 0.06;
/// Fades each burst in and out so it doesn't click.
const FADE_SECONDS: f32 = 0.03;

/// The sound at sample `n`.
fn sample(n: u64) -> f32 {
    let t = n as f32 / SAMPLE_RATE as f32;
    let at = t % PERIOD_SECONDS;
    if at >= ON_SECONDS {
        return 0.0;
    }
    let fade = (at / FADE_SECONDS).min((ON_SECONDS - at) / FADE_SECONDS).min(1.0);
    let tau = std::f32::consts::TAU;
    ((at * 440.0 * tau).sin() + (at * 480.0 * tau).sin()) * 0.5 * VOLUME * fade
}

/// Rings until dropped. If there's no output device it's simply silent.
pub struct Ringer {
    stop: Option<mpsc::Sender<()>>,
}

impl Ringer {
    pub fn start(device: Option<String>) -> Self {
        let (stop, stopped) = mpsc::channel::<()>();
        let _ = std::thread::Builder::new().name("voice-ring".into()).spawn(move || {
            let mut n = 0u64;
            let stream = System.open_output(
                device.as_deref(),
                Box::new(move |out| {
                    debug_assert_eq!(out.len(), FRAME);
                    for s in out.iter_mut() {
                        *s = sample(n);
                        n += 1;
                    }
                }),
            );
            if let Err(e) = &stream {
                tracing::debug!("voice: can't play the ringing sound: {e}");
            }
            // Waits until the `Ringer` is dropped.
            let _ = stopped.recv();
            drop(stream);
        });
        Self { stop: Some(stop) }
    }
}

impl Drop for Ringer {
    fn drop(&mut self) {
        self.stop.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_then_pauses_quietly() {
        let at = |seconds: f32| (seconds * SAMPLE_RATE as f32) as u64;
        let burst: Vec<f32> = (at(0.5)..at(0.6)).map(sample).collect();
        assert!(burst.iter().any(|s| s.abs() > 0.01), "audible while ringing");
        assert!(burst.iter().all(|s| s.abs() <= VOLUME), "and never loud");
        assert!((at(2.5)..at(4.5)).step_by(97).all(|n| sample(n) == 0.0), "silent in the pause");
        assert!((at(5.5)..at(5.6)).any(|n| sample(n).abs() > 0.01), "and it repeats");
    }
}

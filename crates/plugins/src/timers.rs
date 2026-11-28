//! Timers: `schedule_in`, `schedule_every`, `schedule_at`, `cancel_timer`,
//! `timers`, and the `on_timer(id)` hook. No permission needed (like
//! `on_tick`), but bounded: at most [`MAX_TIMERS`], at least
//! [`MIN_DELAY`] seconds away, and all gone when the plugin stops.

use super::{Host, map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

pub const MAX_TIMERS: usize = 20;
/// Seconds: the shortest delay or repeat.
pub const MIN_DELAY: u64 = 5;
/// Seconds: the furthest ahead a timer can be (30 days).
const MAX_AHEAD: u64 = 30 * 24 * 3600;

#[derive(Debug, Clone)]
pub(crate) struct Timer {
    pub id: u64,
    /// Unix seconds.
    pub due: u64,
    /// Seconds between runs for a repeating timer.
    pub every: Option<u64>,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Host {
    fn add_timer(&mut self, due: u64, every: Option<u64>) -> Result<String, Box<EvalAltResult>> {
        if self.timers.len() >= MAX_TIMERS {
            return Err(format!("too many timers: at most {MAX_TIMERS}").into());
        }
        self.next_timer += 1;
        let id = self.next_timer;
        self.timers.push(Timer { id, due, every });
        Ok(id.to_string())
    }

    /// Timers due at `now`, oldest first; repeating ones are set again.
    pub(crate) fn due_timers(&mut self, now: u64) -> Vec<u64> {
        let mut fired = Vec::new();
        self.timers.sort_by_key(|t| t.due);
        self.timers.retain_mut(|t| {
            if t.due > now {
                return true;
            }
            fired.push(t.id);
            match t.every {
                Some(every) => {
                    // Skip runs missed while asleep instead of firing them all.
                    while t.due <= now {
                        t.due += every;
                    }
                    true
                }
                None => false,
            }
        });
        fired
    }

    pub(crate) fn next_timer_due(&self) -> Option<u64> {
        self.timers.iter().map(|t| t.due).min()
    }
}

fn seconds(s: i64) -> Result<u64, Box<EvalAltResult>> {
    if s < MIN_DELAY as i64 {
        return Err(format!("timers need at least {MIN_DELAY} seconds").into());
    }
    Ok((s as u64).min(MAX_AHEAD))
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("schedule_in", move |secs: i64| -> Result<String, Box<EvalAltResult>> {
        let secs = seconds(secs)?;
        h.lock().unwrap().add_timer(now() + secs, None)
    });

    let h = host.clone();
    e.register_fn("schedule_every", move |secs: i64| -> Result<String, Box<EvalAltResult>> {
        let secs = seconds(secs)?;
        h.lock().unwrap().add_timer(now() + secs, Some(secs))
    });

    let h = host.clone();
    e.register_fn("schedule_at", move |when: i64| -> Result<String, Box<EvalAltResult>> {
        let n = now();
        let ahead = (when.max(0) as u64).saturating_sub(n);
        if ahead < MIN_DELAY {
            return Err(format!("schedule_at needs a time at least {MIN_DELAY} seconds from now").into());
        }
        if ahead > MAX_AHEAD {
            return Err("schedule_at can be at most 30 days ahead".into());
        }
        h.lock().unwrap().add_timer(when as u64, None)
    });

    let h = host.clone();
    e.register_fn("cancel_timer", move |id: &str| -> bool {
        let Ok(id) = id.parse::<u64>() else { return false };
        let mut h = h.lock().unwrap();
        let before = h.timers.len();
        h.timers.retain(|t| t.id != id);
        h.timers.len() != before
    });

    let h = host.clone();
    e.register_fn("timers", move || -> Array {
        let h = h.lock().unwrap();
        h.timers
            .iter()
            .map(|t| {
                Dynamic::from_map(map([
                    ("id", t.id.to_string().into()),
                    ("due", (t.due as i64).into()),
                    ("every", t.every.map(|e| Dynamic::from(e as i64)).unwrap_or(Dynamic::UNIT)),
                ]))
            })
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use std::collections::BTreeSet;

    fn run(script: &str) -> crate::Plugin {
        let mut p = plugin("id = \"tm\"\nname = \"T\"\npermissions = [\"storage\"]\n", script);
        p.start(&BTreeSet::from([crate::Permission::Storage]), Default::default());
        p
    }

    #[test]
    fn timers_fire_repeat_and_cancel() {
        let mut p = run(
            r#"
            fn on_load() {
                storage_set("once", schedule_in(10));
                storage_set("every", schedule_every(60));
                let gone = schedule_in(20);
                storage_set("cancelled", "" + cancel_timer(gone) + " " + cancel_timer(gone));
                storage_set("count", "" + timers().len());
            }
            fn on_timer(id) { let n = storage_get("fired") ?? ""; storage_set("fired", n + id + ","); }
            "#,
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["cancelled"], "true false");
        assert_eq!(s["count"], "2");
        let (once, every) = (s["once"].clone(), s["every"].clone());
        let t = now();
        assert!(p.timers(t).is_some_and(|due| due >= t + 9), "nothing due yet");
        assert!(p.take_storage().is_none());
        // 2 minutes later: the one-off once, the repeating one once (missed runs are skipped).
        let next = p.timers(t + 120).unwrap();
        assert_eq!(p.take_storage().unwrap()["fired"], format!("{once},{every},"));
        assert!(next > t + 120 && next <= t + 180, "{next}");
        p.timers(next);
        assert_eq!(p.take_storage().unwrap()["fired"], format!("{once},{every},{every},"));
    }

    #[test]
    fn timers_are_bounded_and_cleared_on_stop() {
        for (call, why) in [("schedule_in(1)", "at least 5"), ("schedule_every(0)", "at least 5"), ("schedule_at(now() + 1)", "at least 5"), ("schedule_at(now() + 99999999)", "30 days")] {
            let p = run(&format!("fn on_load() {{ {call}; }}"));
            assert!(p.error.as_deref().is_some_and(|e| e.contains(why)), "{call}: {:?}", p.error);
        }
        let p = run("fn on_load() { for i in 0..25 { schedule_in(60); } }");
        assert!(p.error.as_deref().is_some_and(|e| e.contains("too many timers")), "{:?}", p.error);

        let mut p = run("fn on_load() { schedule_in(10); } fn on_timer(id) { storage_set(\"x\", id); }");
        p.stop();
        assert!(p.timers(now() + 100).is_none(), "stopping clears timers");
        assert!(p.take_storage().is_none());
    }
}

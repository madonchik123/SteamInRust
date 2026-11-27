//! Downloads: `downloads()` (`downloads.read`), the actions
//! `pause_download`, `resume_download`, `cancel_download`, `move_download`,
//! `download_now` (`downloads.write`), and the download events.

use super::{Host, HostEvent, Permission, Request, map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

/// A running or queued download as plugins see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadInfo {
    pub app_id: u32,
    pub name: String,
    /// 0..1
    pub progress: f32,
    pub bytes_per_second: u64,
    pub paused: bool,
    /// Waiting in the queue (not started).
    pub queued: bool,
    /// "queued", "preparing", "checking", "downloading", "finishing", "verifying".
    pub phase: &'static str,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub error: Option<String>,
}

/// A download phase as plugins see it.
pub fn phase(p: model::DownloadPhase) -> &'static str {
    use model::DownloadPhase as P;
    match p {
        P::Queued => "queued",
        P::Preparing => "preparing",
        P::Checking => "checking",
        P::Downloading => "downloading",
        P::Finishing => "finishing",
        P::Verifying => "verifying",
        P::Done => "done",
        P::Failed => "failed",
    }
}

fn download_map(d: &DownloadInfo, position: Option<usize>) -> Dynamic {
    Dynamic::from_map(map([
        ("app_id", (d.app_id as i64).into()),
        ("name", d.name.clone().into()),
        ("progress", (d.progress as f64).into()),
        ("bytes_per_second", (d.bytes_per_second as i64).into()),
        ("paused", d.paused.into()),
        ("queued", d.queued.into()),
        ("phase", d.phase.into()),
        ("bytes_done", (d.bytes_done as i64).into()),
        ("bytes_total", (d.bytes_total as i64).into()),
        ("error", d.error.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT)),
        ("position", position.map(|p| Dynamic::from(p as i64)).unwrap_or(Dynamic::UNIT)),
    ]))
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("downloads", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::DownloadsRead)?;
        let mut place = 0;
        Ok(h.snapshot
            .downloads
            .iter()
            .map(|d| {
                let pos = d.queued.then(|| {
                    place += 1;
                    place - 1
                });
                download_map(d, pos)
            })
            .collect())
    });

    // Each action checks the download exists and is in a state it makes sense for.
    let h = host.clone();
    let act = move |app_id: i64, check: fn(&DownloadInfo) -> Result<(), &'static str>, r: Request| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::DownloadsWrite)?;
        let d = h.snapshot.downloads.iter().find(|d| d.app_id as i64 == app_id).ok_or("no download for that game")?;
        check(d)?;
        h.push(r)
    };
    let a = act.clone();
    e.register_fn("pause_download", move |app_id: i64| {
        a(app_id, |d| if d.queued || d.phase == "verifying" { Err("only a running download can be paused") } else { Ok(()) }, Request::PauseDownload(app_id as u32))
    });
    let a = act.clone();
    e.register_fn("resume_download", move |app_id: i64| a(app_id, |d| if d.paused { Ok(()) } else { Err("that download isn't paused") }, Request::ResumeDownload(app_id as u32)));
    let a = act.clone();
    e.register_fn("cancel_download", move |app_id: i64| {
        a(app_id, |d| if d.phase == "verifying" { Err("that's a file check, not a download") } else { Ok(()) }, Request::CancelDownload(app_id as u32))
    });
    let a = act.clone();
    e.register_fn("move_download", move |app_id: i64, to: i64| {
        a(app_id, |d| if d.queued { Ok(()) } else { Err("only a queued download can be moved") }, Request::MoveDownload(app_id as u32, to.max(0) as usize))
    });
    let a = act;
    e.register_fn("download_now", move |app_id: i64| a(app_id, |d| if d.queued { Ok(()) } else { Err("that download isn't queued") }, Request::DownloadNow(app_id as u32)));
}

/// Turns the app's download updates into `download_started`,
/// `download_paused` and `download_resumed` events.
#[derive(Debug, Default)]
pub struct DownloadWatch {
    /// Downloads seen running, and whether they were paused.
    seen: std::collections::HashMap<u32, bool>,
}

impl DownloadWatch {
    pub fn update(&mut self, d: &model::DownloadStatus) -> Option<HostEvent> {
        use model::DownloadPhase as P;
        let app_id = d.app_id;
        if matches!(d.phase, P::Done | P::Failed | P::Verifying) {
            self.seen.remove(&app_id);
            return None;
        }
        if d.phase == P::Queued && !self.seen.contains_key(&app_id) {
            return None;
        }
        match self.seen.insert(app_id, d.paused) {
            None => Some(HostEvent::DownloadStarted { app_id }),
            Some(false) if d.paused => Some(HostEvent::DownloadPaused { app_id }),
            Some(true) if !d.paused => Some(HostEvent::DownloadResumed { app_id }),
            _ => None,
        }
    }

    /// A download finished or was cancelled.
    pub fn forget(&mut self, app_id: u32) {
        self.seen.remove(&app_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use crate::Snapshot;
    use std::collections::BTreeSet;

    fn snap() -> Snapshot {
        Snapshot {
            downloads: vec![
                DownloadInfo { app_id: 440, name: "TF2".into(), progress: 0.5, bytes_per_second: 1000, phase: "downloading", bytes_done: 50, bytes_total: 100, ..Default::default() },
                DownloadInfo { app_id: 620, name: "Portal 2".into(), paused: true, phase: "downloading", ..Default::default() },
                DownloadInfo { app_id: 70, name: "HL".into(), queued: true, phase: "queued", ..Default::default() },
                DownloadInfo { app_id: 80, name: "CS".into(), phase: "verifying", ..Default::default() },
            ],
            ..Default::default()
        }
    }

    fn run(script: &str, perms: &[Permission]) -> crate::Plugin {
        let keys: Vec<String> = perms.iter().map(|p| format!("\"{}\"", p.key())).collect();
        let mut p = plugin(&format!("id = \"dl\"\nname = \"D\"\npermissions = [{}]\n", keys.join(", ")), script);
        p.set_snapshot(snap());
        p.start(&perms.iter().copied().collect::<BTreeSet<_>>(), Default::default());
        p
    }

    #[test]
    fn downloads_have_bytes_phase_and_queue_place() {
        let p = run(
            r#"fn on_load() {
                let d = downloads();
                storage_set("d", d[0].phase + " " + d[0].bytes_done + "/" + d[0].bytes_total + " " + (d[0].position == ()) + " " + d[2].position + " " + (d[0].error == ()));
            }"#,
            &[Permission::DownloadsRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.take_storage().unwrap()["d"], "downloading 50/100 true 0 true");
    }

    #[test]
    fn actions_go_to_the_app() {
        let p = run(
            "fn on_load() { pause_download(440); resume_download(620); move_download(70, 0); download_now(70); cancel_download(440); }",
            &[Permission::DownloadsWrite],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert!(matches!(
            reqs.as_slice(),
            [Request::PauseDownload(440), Request::ResumeDownload(620), Request::MoveDownload(70, 0), Request::DownloadNow(70), Request::CancelDownload(440)]
        ));
    }

    #[test]
    fn actions_are_checked() {
        for (call, why) in [
            ("pause_download(1)", "no download"),
            ("pause_download(70)", "running"),
            ("resume_download(440)", "isn't paused"),
            ("cancel_download(80)", "file check"),
            ("move_download(440, 1)", "queued"),
            ("download_now(440)", "isn't queued"),
        ] {
            let p = run(&format!("fn on_load() {{ {call}; }}"), &[Permission::DownloadsWrite]);
            assert!(p.error.as_deref().is_some_and(|e| e.contains(why)), "{call}: {:?}", p.error);
            assert!(p.drain().is_empty());
        }
        // Reading isn't enough to act.
        let p = run("fn on_load() { pause_download(440); }", &[Permission::DownloadsRead]);
        assert!(p.error.as_deref().is_some_and(|e| e.contains("downloads.write")), "{:?}", p.error);
    }

    #[test]
    fn events_for_start_pause_and_resume() {
        use model::{DownloadPhase as P, DownloadStatus};
        let st = |phase, paused| DownloadStatus { app_id: 440, phase, paused, ..Default::default() };
        let mut w = DownloadWatch::default();
        assert!(w.update(&st(P::Queued, false)).is_none());
        assert!(matches!(w.update(&st(P::Preparing, false)), Some(HostEvent::DownloadStarted { app_id: 440 })));
        assert!(w.update(&st(P::Downloading, false)).is_none());
        assert!(matches!(w.update(&st(P::Downloading, true)), Some(HostEvent::DownloadPaused { app_id: 440 })));
        assert!(matches!(w.update(&st(P::Downloading, false)), Some(HostEvent::DownloadResumed { app_id: 440 })));
        assert!(w.update(&st(P::Done, false)).is_none());
        assert!(w.update(&st(P::Verifying, false)).is_none(), "file checks aren't downloads");

        let mut p = run("fn on_event(name, data) { storage_set(name, \"\" + data.app_id); }", &[Permission::DownloadsRead, Permission::Storage]);
        p.event(&HostEvent::DownloadPaused { app_id: 440 });
        p.event(&HostEvent::DownloadFinished { app_id: 440, ok: false, error: Some("disk full".into()) });
        let s = p.take_storage().unwrap();
        assert_eq!(s["download_paused"], "440");
        assert_eq!(s["download_finished"], "440");
    }
}

//! Achievements: `achievement_progress`, `achievements`, `achievement_info`
//! (`achievements.read`), and the `achievement_unlocked` event.

use super::{Host, HostEvent, Permission, Request, map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn achievement_map(a: &model::Achievement) -> Dynamic {
    Dynamic::from_map(map([
        ("name", a.name.clone().into()),
        ("description", a.description.clone().into()),
        ("icon", a.icon.clone().into()),
        ("icon_gray", a.icon_gray.clone().into()),
        ("hidden", a.hidden.into()),
        ("unlocked", a.unlocked.into()),
        ("unlock_time", (a.unlock_time as i64).into()),
        ("global_percent", (a.global_percent as f64).into()),
    ]))
}

type Lists = Arc<std::collections::HashMap<u32, Result<Vec<model::Achievement>, String>>>;

/// A game's loaded achievements, read from the shared snapshot (empty if the
/// game has none or Steam refused).
struct List(Lists, u32);

impl std::ops::Deref for List {
    type Target = [model::Achievement];
    fn deref(&self) -> &Self::Target {
        match self.0.get(&self.1) {
            Some(Ok(l)) => l,
            _ => &[],
        }
    }
}

/// The achievements of `app_id` once loaded, or `None` after asking for them.
fn list(h: &mut Host, app_id: u32) -> Option<List> {
    let all = h.snapshot.achievements.clone();
    if all.contains_key(&app_id) {
        return Some(List(all, app_id));
    }
    h.push_load(Request::LoadAchievements(app_id));
    None
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("achievement_progress", move |app_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::AchievementsRead)?;
        let id = app_id as u32;
        Ok(match h.snapshot.achievement_progress.get(&id).copied() {
            Some((done, total)) => Dynamic::from_map(map([
                ("unlocked", (done as i64).into()),
                ("total", (total as i64).into()),
                ("percent", (if total == 0 { 0.0 } else { done as f64 * 100.0 / total as f64 }).into()),
            ])),
            None => {
                h.push_load(Request::LoadAchievementProgress(vec![id]));
                Dynamic::UNIT
            }
        })
    });

    let h = host.clone();
    e.register_fn("achievements", move |app_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::AchievementsRead)?;
        Ok(match list(&mut h, app_id as u32) {
            Some(l) => Dynamic::from_array(l.iter().map(achievement_map).collect::<Array>()),
            None => Dynamic::UNIT,
        })
    });

    let h = host.clone();
    e.register_fn("achievement_info", move |app_id: i64, name: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::AchievementsRead)?;
        Ok(list(&mut h, app_id as u32).and_then(|l| l.iter().find(|a| a.name == name).map(achievement_map)).unwrap_or(Dynamic::UNIT))
    });
}

/// Watches a game's achievement list for new unlocks. The first list seen is
/// the starting point; later lists give one event per newly unlocked one.
#[derive(Debug, Default)]
pub struct AchievementWatch {
    seen: std::collections::HashMap<u32, HashSet<String>>,
}

impl AchievementWatch {
    pub fn update(&mut self, app_id: u32, list: &[model::Achievement]) -> Vec<HostEvent> {
        let now: HashSet<String> = list.iter().filter(|a| a.unlocked).map(|a| a.name.clone()).collect();
        let Some(before) = self.seen.insert(app_id, now) else { return Vec::new() };
        list.iter()
            .filter(|a| a.unlocked && !before.contains(&a.name))
            .map(|a| HostEvent::AchievementUnlocked { app_id, name: a.name.clone(), description: a.description.clone(), unlock_time: a.unlock_time })
            .collect()
    }

    /// Whether a starting point exists for this game.
    pub fn knows(&self, app_id: u32) -> bool {
        self.seen.contains_key(&app_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use crate::Snapshot;
    use std::collections::{BTreeSet, HashMap};

    fn ach(name: &str, unlocked: bool) -> model::Achievement {
        model::Achievement { name: name.into(), description: format!("Do {name}"), unlocked, unlock_time: if unlocked { 100 } else { 0 }, global_percent: 12.5, ..Default::default() }
    }

    fn snap() -> Snapshot {
        Snapshot {
            achievement_progress: Arc::new(HashMap::from([(440, (3, 4))])),
            achievements: Arc::new(HashMap::from([(440, Ok(vec![ach("First", true), ach("Second", false)])), (70, Err("no stats".into()))])),
            ..Default::default()
        }
    }

    fn run(script: &str, perms: &[Permission]) -> crate::Plugin {
        let keys: Vec<String> = perms.iter().map(|p| format!("\"{}\"", p.key())).collect();
        let mut p = plugin(&format!("id = \"ach\"\nname = \"A\"\npermissions = [{}]\n", keys.join(", ")), script);
        p.set_snapshot(snap());
        p.start(&perms.iter().copied().collect::<BTreeSet<_>>(), Default::default());
        p
    }

    #[test]
    fn reads_progress_lists_and_one_achievement() {
        let p = run(
            r#"fn on_load() {
                let pr = achievement_progress(440);
                storage_set("p", pr.unlocked + "/" + pr.total + " " + pr.percent);
                let l = achievements(440);
                storage_set("l", l.len() + " " + l[0].name + " " + l[0].unlocked + " " + l[1].unlock_time + " " + l[0].global_percent);
                storage_set("i", achievement_info(440, "Second").description);
                storage_set("none", "" + (achievement_info(440, "Nope") == ()));
                storage_set("err", "" + achievements(70).len());
            }"#,
            &[Permission::AchievementsRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["p"], "3/4 75.0");
        assert_eq!(s["l"], "2 First true 0 12.5");
        assert_eq!(s["i"], "Do Second");
        assert_eq!(s["none"], "true");
        assert_eq!(s["err"], "0");
        assert!(p.drain().is_empty(), "nothing to load");
    }

    #[test]
    fn unknown_games_load_once_per_call() {
        let p = run(
            r#"fn on_load() {
                for i in 0..50 { if achievements(10) != () { throw "loaded?"; } }
                achievement_info(20, "x");
                achievement_progress(30);
                achievement_progress(40);
            }"#,
            &[Permission::AchievementsRead],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert_eq!(reqs.iter().filter(|r| matches!(r, Request::LoadAchievements(10))).count(), 1);
        assert!(reqs.iter().any(|r| matches!(r, Request::LoadAchievements(20))));
        assert!(reqs.iter().any(|r| matches!(r, Request::LoadAchievementProgress(ids) if ids == &vec![30, 40])));
    }

    #[test]
    fn achievements_need_permission() {
        for call in ["achievement_progress(440)", "achievements(440)", "achievement_info(440, \"First\")"] {
            let p = run(&format!("fn on_load() {{ {call}; }}"), &[Permission::LibraryRead]);
            assert!(p.error.as_deref().is_some_and(|e| e.contains("achievements.read")), "{call}: {:?}", p.error);
        }
    }

    #[test]
    fn unlock_event_only_for_new_unlocks() {
        let mut w = AchievementWatch::default();
        assert!(w.update(440, &[ach("First", true), ach("Second", false)]).is_empty(), "the first list is the start");
        assert!(w.knows(440));
        let ev = w.update(440, &[ach("First", true), ach("Second", true)]);
        assert!(matches!(ev.as_slice(), [HostEvent::AchievementUnlocked { app_id: 440, name, .. }] if name == "Second"));
        assert!(w.update(440, &[ach("First", true), ach("Second", true)]).is_empty());

        // Delivered with the data, only with the permission.
        let mut p = run("fn on_event(name, data) { storage_set(name, data.name + \" \" + data.app_id); }", &[Permission::AchievementsRead, Permission::Storage]);
        p.event(&ev[0]);
        assert_eq!(p.take_storage().unwrap()["achievement_unlocked"], "Second 440");
        let mut q = run("fn on_event(name, data) { storage_set(name, \"x\"); }", &[Permission::Storage]);
        q.event(&ev[0]);
        assert!(q.take_storage().is_none());
    }
}

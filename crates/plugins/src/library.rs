//! Library info: `game_info`, `installed_games`, `recent_games` (`library.read`).

use super::{Host, Permission, map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

/// A game in your library as plugins see it: owned (from Steam), installed
/// (found on disk) or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameInfo {
    pub app_id: u32,
    pub name: String,
    pub installed: bool,
    /// On the account (not just found on disk).
    pub owned: bool,
    pub playtime_minutes: u64,
    pub playtime_2weeks: Option<u64>,
    /// Unix seconds.
    pub last_played: Option<u64>,
    pub size_on_disk: Option<u64>,
    /// Unix seconds.
    pub last_updated: Option<u64>,
    pub update_required: bool,
    /// Steam's app type ("game", "application", "tool"...).
    pub kind: String,
    /// Steam genre ids.
    pub genres: Vec<u32>,
    /// Steam feature category ids (22 achievements, 23 cloud...).
    pub categories: Vec<u32>,
    /// Steam Deck: 0 unknown, 1 unsupported, 2 playable, 3 verified.
    pub deck: u8,
    /// "windows,macos,linux", empty when not known.
    pub os_list: String,
    /// Icon hash.
    pub icon: Option<String>,
}

fn deck(d: u8) -> &'static str {
    match d {
        1 => "unsupported",
        2 => "playable",
        3 => "verified",
        _ => "unknown",
    }
}

fn opt(v: Option<u64>) -> Dynamic {
    v.map(|x| Dynamic::from(x as i64)).unwrap_or(Dynamic::UNIT)
}

fn ids(v: &[u32]) -> Dynamic {
    Dynamic::from_array(v.iter().map(|x| Dynamic::from(*x as i64)).collect())
}

/// One game as a Rhai map.
pub(crate) fn game_map(g: &GameInfo) -> Dynamic {
    let id = g.app_id;
    let art = map([
        ("header", model::art::header(id).into()),
        ("capsule", model::art::capsule(id).into()),
        ("hero", model::art::hero(id).into()),
        ("logo", model::art::logo(id).into()),
        ("icon", g.icon.as_deref().filter(|h| !h.is_empty()).map(|h| Dynamic::from(model::art::icon(id, h))).unwrap_or(Dynamic::UNIT)),
    ]);
    let platforms: Array = g.os_list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| Dynamic::from(s.to_string())).collect();
    Dynamic::from_map(map([
        ("app_id", (id as i64).into()),
        ("name", g.name.clone().into()),
        ("installed", g.installed.into()),
        ("owned", g.owned.into()),
        ("playtime_minutes", (g.playtime_minutes as i64).into()),
        ("playtime_2weeks", opt(g.playtime_2weeks)),
        ("last_played", opt(g.last_played)),
        ("size_on_disk", opt(g.size_on_disk)),
        ("last_updated", opt(g.last_updated)),
        ("update_required", g.update_required.into()),
        ("type", g.kind.clone().into()),
        ("genres", ids(&g.genres)),
        ("categories", ids(&g.categories)),
        ("deck", deck(g.deck).into()),
        ("platforms", Dynamic::from_array(platforms)),
        ("store_url", format!("https://store.steampowered.com/app/{id}/").into()),
        ("art", Dynamic::from_map(art)),
    ]))
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("game_info", move |app_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::LibraryRead)?;
        Ok(h.snapshot.library.iter().find(|g| g.app_id as i64 == app_id).map(game_map).unwrap_or(Dynamic::UNIT))
    });

    let h = host.clone();
    e.register_fn("installed_games", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::LibraryRead)?;
        Ok(h.snapshot.library.iter().filter(|g| g.installed).map(game_map).collect())
    });

    let h = host.clone();
    e.register_fn("recent_games", move |count: i64| -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::LibraryRead)?;
        let mut played: Vec<&GameInfo> = h.snapshot.library.iter().filter(|g| g.last_played.is_some_and(|t| t > 0)).collect();
        played.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        Ok(played.into_iter().take(count.clamp(1, 100) as usize).map(game_map).collect())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use crate::{Request, Snapshot};
    use std::collections::BTreeSet;

    fn library() -> Arc<Vec<GameInfo>> {
        Arc::new(vec![
            GameInfo { app_id: 440, name: "TF2".into(), installed: true, owned: true, playtime_minutes: 90, last_played: Some(200), size_on_disk: Some(5_000), os_list: "windows,linux".into(), deck: 3, genres: vec![1], icon: Some("abc".into()), ..Default::default() },
            GameInfo { app_id: 620, name: "Portal 2".into(), owned: true, last_played: Some(500), ..Default::default() },
            GameInfo { app_id: 70, name: "Half-Life".into(), installed: true, owned: true, ..Default::default() },
        ])
    }

    fn run(script: &str, perms: &[Permission]) -> crate::Plugin {
        let keys: Vec<String> = perms.iter().map(|p| format!("\"{}\"", p.key())).collect();
        let mut p = plugin(&format!("id = \"lib\"\nname = \"L\"\npermissions = [{}]\n", keys.join(", ")), script);
        p.set_snapshot(Snapshot { library: library(), ..Default::default() });
        p.start(&perms.iter().copied().collect::<BTreeSet<_>>(), Default::default());
        p
    }

    #[test]
    fn game_info_has_details_and_art() {
        let p = run(
            r#"fn on_load() {
                let g = game_info(440);
                storage_set("g", g.name + " " + g.size_on_disk + " " + g.deck + " " + g.platforms + " " + g.store_url + " " + g.art.icon.contains("abc") + " " + g.genres);
                storage_set("none", "" + (game_info(1) == ()));
                storage_set("unset", "" + (game_info(620).size_on_disk == ()));
            }"#,
            &[Permission::LibraryRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["g"], "TF2 5000 verified [\"windows\", \"linux\"] https://store.steampowered.com/app/440/ true [1]");
        assert_eq!(s["none"], "true");
        assert_eq!(s["unset"], "true");
    }

    #[test]
    fn installed_and_recent_games() {
        let p = run(
            r#"fn on_load() {
                storage_set("i", installed_games().map(|g| g.app_id).to_string());
                storage_set("r", recent_games(5).map(|g| g.app_id).to_string());
                storage_set("r1", recent_games(1).map(|g| g.app_id).to_string());
            }"#,
            &[Permission::LibraryRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["i"], "[440, 70]");
        assert_eq!(s["r"], "[620, 440]");
        assert_eq!(s["r1"], "[620]");
    }

    #[test]
    fn library_calls_need_permission() {
        for call in ["game_info(440)", "installed_games()", "recent_games(3)"] {
            let p = run(&format!("fn on_load() {{ {call}; }}"), &[]);
            assert!(p.error.as_deref().is_some_and(|e| e.contains("library.read")), "{call}: {:?}", p.error);
            assert!(p.drain().iter().all(|r| !matches!(r, Request::Notify(_))));
        }
    }

    #[test]
    fn api_version_is_there_without_permissions() {
        let p = run("fn on_load() { if api_version() < 2 { throw \"old\"; } }", &[]);
        assert!(p.error.is_none(), "{:?}", p.error);
    }
}

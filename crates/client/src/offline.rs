//! What offline mode needs, kept on disk per account (under
//! `<config>/offline/<account id>/`), like Steam's own client does:
//!
//! - `account.json`: the last successful sign-in (name, avatar, country,
//!   owned apps from PICS with launch options and cloud settings, friends).
//! - `playtime.json`: play sessions while not connected to Steam.
//! - `cloud-pending.json`: games whose saves still need uploading.
//! - `stats/<app>.json`: a game's stats and achievements, with changes made
//!   offline that still need sending.
//!
//! Nothing here is a credential; the refresh token stays in `accounts.json`.

use crate::ipc::protocol::Stats;
use model::{AppId, OwnedApp, Persona, Relationship, SteamId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Send play time and stats recorded offline to Steam once back online
/// (`Player.RecordDisconnectedPlaytime`, `ClientStoreUserStats2`), as
/// Steam's own client does.
pub const SEND_TO_STEAM: bool = true;

fn dir(steam_id: SteamId) -> PathBuf {
    crate::store::config_dir().join("offline").join(model::account_id(steam_id).to_string())
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Write through a temporary file so a crash never leaves half a file.
fn write<T: Serialize>(path: &Path, value: &T) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let Ok(json) = serde_json::to_string(value) else { return };
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        if let Err(e) = std::fs::rename(&tmp, path) {
            tracing::warn!("couldn't save {}: {e}", path.display());
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The last successful sign-in, enough to show the library offline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountCache {
    pub steam_id: SteamId,
    pub account: String,
    pub persona_name: String,
    pub avatar_hash: Option<String>,
    pub country: String,
    pub apps: Vec<OwnedApp>,
    pub friends: Vec<(SteamId, Relationship)>,
    /// Friends' names and avatars (their status isn't kept: it's unknown offline).
    pub personas: Vec<Persona>,
    /// Unix seconds of the snapshot.
    pub saved: u64,
}

pub fn load_account(steam_id: SteamId) -> Option<AccountCache> {
    read(&dir(steam_id).join("account.json"))
}

pub fn save_account(cache: &AccountCache) {
    if cache.steam_id == 0 || cache.apps.is_empty() {
        return; // Never replace a good snapshot with an empty one.
    }
    write(&dir(cache.steam_id).join("account.json"), cache);
}

/// One stretch of play while not connected to Steam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaySession {
    pub app_id: AppId,
    /// Unix seconds.
    pub start: u64,
    pub seconds: u64,
    /// Played in offline mode (`false`: the connection dropped while playing).
    pub offline: bool,
    /// Sent to Steam since.
    #[serde(default)]
    pub reported: bool,
}

fn playtime_path(steam_id: SteamId) -> PathBuf {
    dir(steam_id).join("playtime.json")
}

pub fn play_sessions(steam_id: SteamId) -> Vec<PlaySession> {
    read(&playtime_path(steam_id)).unwrap_or_default()
}

pub fn record_play(steam_id: SteamId, s: PlaySession) {
    if s.seconds == 0 {
        return;
    }
    let mut list = play_sessions(steam_id);
    list.push(s);
    write(&playtime_path(steam_id), &list);
}

/// Mark these sessions as sent to Steam.
pub fn mark_reported(steam_id: SteamId, sent: &[PlaySession]) {
    let mut list = play_sessions(steam_id);
    for s in list.iter_mut() {
        if sent.iter().any(|x| x.app_id == s.app_id && x.start == s.start && x.seconds == s.seconds) {
            s.reported = true;
        }
    }
    write(&playtime_path(steam_id), &list);
}

/// Per game: (seconds played while not connected, of those not on Steam yet).
pub fn playtime_totals(sessions: &[PlaySession]) -> Vec<(AppId, u64, u64)> {
    let mut by_app: BTreeMap<AppId, (u64, u64)> = BTreeMap::new();
    for s in sessions {
        let e = by_app.entry(s.app_id).or_default();
        e.0 += s.seconds;
        if !s.reported {
            e.1 += s.seconds;
        }
    }
    by_app.into_iter().map(|(a, (total, pending))| (a, total, pending)).collect()
}

/// Games whose saves changed while offline: (app, install folder).
fn cloud_path(steam_id: SteamId) -> PathBuf {
    dir(steam_id).join("cloud-pending.json")
}

pub fn cloud_pending(steam_id: SteamId) -> Vec<(AppId, PathBuf)> {
    read(&cloud_path(steam_id)).unwrap_or_default()
}

pub fn add_cloud_pending(steam_id: SteamId, app_id: AppId, install_dir: &Path) {
    let mut list = cloud_pending(steam_id);
    list.retain(|(a, _)| *a != app_id);
    list.push((app_id, install_dir.to_path_buf()));
    write(&cloud_path(steam_id), &list);
}

pub fn remove_cloud_pending(steam_id: SteamId, app_id: AppId) {
    let mut list = cloud_pending(steam_id);
    let before = list.len();
    list.retain(|(a, _)| *a != app_id);
    if list.len() != before {
        write(&cloud_path(steam_id), &list);
    }
}

/// A game's stats as last loaded from Steam, plus changes made offline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GameStats {
    pub stats: Option<Stats>,
    /// Stat id -> value, stored by the game while offline.
    pub pending: BTreeMap<u32, u32>,
    /// The game asked for a reset while offline.
    pub reset: bool,
}

fn stats_path(steam_id: SteamId, app_id: AppId) -> PathBuf {
    dir(steam_id).join("stats").join(format!("{app_id}.json"))
}

pub fn load_stats(steam_id: SteamId, app_id: AppId) -> GameStats {
    read(&stats_path(steam_id, app_id)).unwrap_or_default()
}

pub fn save_stats(steam_id: SteamId, app_id: AppId, s: &GameStats) {
    write(&stats_path(steam_id, app_id), s);
}

/// Remember stats Steam just sent (keeps pending offline changes).
pub fn remember_stats(steam_id: SteamId, app_id: AppId, stats: &Stats) {
    let mut g = load_stats(steam_id, app_id);
    g.stats = Some(stats.clone());
    save_stats(steam_id, app_id, &g);
}

/// The game stored stats while offline: keep them for later and show them
/// to the game next time it starts offline.
pub fn store_offline(steam_id: SteamId, app_id: AppId, values: &[(u32, u32)], reset: bool) {
    let mut g = load_stats(steam_id, app_id);
    if reset {
        g.reset = true;
        g.pending.clear();
    }
    for &(id, v) in values {
        g.pending.insert(id, v);
    }
    save_stats(steam_id, app_id, &g);
}

/// Cached stats with offline changes applied, for a game starting offline.
pub fn offline_view(g: &GameStats) -> Option<Stats> {
    let mut s = g.stats.clone()?;
    if g.reset {
        for st in &mut s.stats {
            st.value = st.default;
        }
        for a in &mut s.achievements {
            a.unlocked = false;
            a.unlock_time = 0;
        }
    }
    for st in &mut s.stats {
        if let Some(v) = g.pending.get(&st.id) {
            st.value = *v;
        }
    }
    for a in &mut s.achievements {
        if let Some(bits) = g.pending.get(&a.block) {
            let on = (bits >> a.bit) & 1 == 1;
            if on && !a.unlocked {
                a.unlock_time = now() as u32;
            }
            a.unlocked = on;
        }
    }
    Some(s)
}

/// Every game with stats waiting to be sent.
pub fn stats_pending(steam_id: SteamId) -> Vec<AppId> {
    let Ok(entries) = std::fs::read_dir(dir(steam_id).join("stats")) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.path().file_stem()?.to_str()?.parse::<AppId>().ok())
        .filter(|app| {
            let g = load_stats(steam_id, *app);
            !g.pending.is_empty() || g.reset
        })
        .collect()
}

/// The pending changes were sent: forget them.
pub fn stats_sent(steam_id: SteamId, app_id: AppId) {
    let mut g = load_stats(steam_id, app_id);
    g.pending.clear();
    g.reset = false;
    save_stats(steam_id, app_id, &g);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::protocol::{Achievement, Stat};

    #[test]
    fn totals_split_reported() {
        let s = |app, secs, reported| PlaySession { app_id: app, start: 1, seconds: secs, offline: true, reported };
        let t = playtime_totals(&[s(10, 60, true), s(10, 120, false), s(20, 30, false)]);
        assert_eq!(t, vec![(10, 180, 120), (20, 30, 30)]);
    }

    #[test]
    fn offline_changes_show_on_top_of_cache() {
        let g = GameStats {
            stats: Some(Stats {
                crc: 1,
                stats: vec![Stat { id: 1, name: "Wins".into(), kind: 1, value: 3, ..Default::default() }, Stat { id: 21, kind: 4, ..Default::default() }],
                achievements: vec![Achievement { name: "A".into(), block: 21, bit: 1, ..Default::default() }],
            }),
            pending: [(1, 5), (21, 0b10)].into_iter().collect(),
            reset: false,
        };
        let v = offline_view(&g).unwrap();
        assert_eq!(v.stats[0].value, 5);
        assert!(v.achievements[0].unlocked);
        assert!(offline_view(&GameStats::default()).is_none());
    }
}

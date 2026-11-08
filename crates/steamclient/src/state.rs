//! Everything the DLL remembers: the session from SteamInRust, queued
//! callbacks, finished API calls, stats and achievements.

use crate::protocol::{Achievement, Reply, Request, Session, Stat, Stats};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Instant;

/// Callback ids (k_iCallback values) used here.
pub mod cb {
    pub const API_CALL_COMPLETED: i32 = 703;
    pub const USER_STATS_RECEIVED: i32 = 1101;
    pub const USER_STATS_STORED: i32 = 1102;
    pub const USER_ACHIEVEMENT_STORED: i32 = 1103;
    pub const NUMBER_OF_CURRENT_PLAYERS: i32 = 1107;
    pub const LEADERBOARD_UGC_SET: i32 = 1111;
    pub const GLOBAL_STATS_RECEIVED: i32 = 1112;
    pub const ENCRYPTED_APP_TICKET_RESPONSE: i32 = 154;
    pub const VALIDATE_AUTH_TICKET_RESPONSE: i32 = 143;
    pub const GET_AUTH_SESSION_TICKET_RESPONSE: i32 = 163;
    pub const GET_TICKET_FOR_WEB_API_RESPONSE: i32 = 168;
    pub const REMOTE_STORAGE_FILE_SHARE_RESULT: i32 = 1307;
    pub const REMOTE_STORAGE_FILE_WRITE_ASYNC_COMPLETE: i32 = 1331;
    pub const REMOTE_STORAGE_FILE_READ_ASYNC_COMPLETE: i32 = 1332;
}

pub const RESULT_OK: i32 = 1;
pub const RESULT_FAIL: i32 = 2;
pub const RESULT_NO_CONNECTION: i32 = 3;
pub const RESULT_FILE_NOT_FOUND: i32 = 9;

pub struct Call {
    pub callback: i32,
    pub data: Vec<u8>,
    pub failed: bool,
}

pub struct StatEntry {
    pub def: Stat,
    pub value: u32,
    /// Value Steam has (last loaded or stored).
    pub stored: u32,
}

pub struct AchEntry {
    pub def: Achievement,
    pub unlocked: bool,
    pub time: u32,
    /// Unlocked state Steam has.
    pub stored: bool,
}

#[derive(Default)]
pub struct StatsState {
    pub loaded: bool,
    pub crc: u32,
    pub stats: Vec<StatEntry>,
    pub achs: Vec<AchEntry>,
    pub storing: bool,
    pub store_again: bool,
    pub reset: bool,
}

impl StatsState {
    pub fn load(&mut self, s: &Stats) {
        self.loaded = true;
        self.crc = s.crc;
        self.stats = s.stats.iter().map(|d| StatEntry { def: d.clone(), value: d.value, stored: d.value }).collect();
        self.achs = s.achievements.iter().map(|a| AchEntry { def: a.clone(), unlocked: a.unlocked, time: a.unlock_time, stored: a.unlocked }).collect();
        // Every achievement needs its block stat to set bits in.
        for a in &s.achievements {
            if !self.stats.iter().any(|x| x.def.id == a.block) {
                let def = Stat { id: a.block, name: String::new(), kind: 4, ..Default::default() };
                self.stats.push(StatEntry { def, value: 0, stored: 0 });
            }
        }
        for a in s.achievements.iter().filter(|a| a.unlocked) {
            if let Some(b) = self.stats.iter_mut().find(|x| x.def.id == a.block) {
                let mask = 1u32.checked_shl(a.bit).unwrap_or(0);
                b.value |= mask;
                b.stored |= mask;
            }
        }
    }

    pub fn stat(&mut self, name: &str) -> Option<&mut StatEntry> {
        let i = self
            .stats
            .iter()
            .position(|s| s.def.kind != 4 && s.def.name == name)
            .or_else(|| self.stats.iter().position(|s| s.def.kind != 4 && s.def.name.eq_ignore_ascii_case(name)))?;
        self.stats.get_mut(i)
    }

    pub fn ach(&self, name: &str) -> Option<usize> {
        self.achs.iter().position(|a| a.def.name == name).or_else(|| self.achs.iter().position(|a| a.def.name.eq_ignore_ascii_case(name)))
    }

    /// Set or clear an achievement, keeping its block stat in step.
    pub fn set_ach(&mut self, i: usize, on: bool) {
        let (block, bit) = (self.achs[i].def.block, self.achs[i].def.bit);
        self.achs[i].unlocked = on;
        if on && self.achs[i].time == 0 {
            self.achs[i].time = now();
        }
        if !on {
            self.achs[i].time = 0;
        }
        if let Some(b) = self.stats.iter_mut().find(|s| s.def.id == block) {
            let mask = 1u32.checked_shl(bit).unwrap_or(0);
            if on { b.value |= mask } else { b.value &= !mask }
        }
    }
}

pub struct Global {
    pub session: Option<Session>,
    pub hello_tried: bool,
    pub started: Instant,
    pub queue: VecDeque<(i32, Vec<u8>)>,
    pub current: Option<Vec<u8>>,
    pub calls: HashMap<u64, Call>,
    pub next_call: u64,
    pub stats: StatsState,
    pub other_stats: HashMap<u64, Stats>,
    pub rich: Vec<(String, String)>,
    strings: HashSet<Box<CStr>>,
    pub streams: HashMap<u64, (String, Vec<u8>)>,
    pub next_handle: u64,
    pub read_results: HashMap<u64, Vec<u8>>,
    pub files: Vec<(CString, i32)>,
    /// Our auth tickets: handle -> CRC.
    pub tickets: HashMap<u32, u32>,
    pub encrypted_ticket: Option<Vec<u8>>,
}

static G: LazyLock<Mutex<Global>> = LazyLock::new(|| {
    Mutex::new(Global {
        session: None,
        hello_tried: false,
        started: Instant::now(),
        queue: VecDeque::new(),
        current: None,
        calls: HashMap::new(),
        next_call: 1,
        stats: StatsState::default(),
        other_stats: HashMap::new(),
        rich: Vec::new(),
        strings: HashSet::new(),
        streams: HashMap::new(),
        next_handle: 1,
        read_results: HashMap::new(),
        files: Vec::new(),
        tickets: HashMap::new(),
        encrypted_ticket: None,
    })
});

pub fn lock() -> MutexGuard<'static, Global> {
    G.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

impl Global {
    /// A NUL-terminated copy that stays valid for the life of the process
    /// (Steam's string getters return pointers the game may keep).
    pub fn intern(&mut self, s: &str) -> *const c_char {
        let c = CString::new(s.replace('\0', "")).unwrap_or_default();
        if let Some(existing) = self.strings.get(c.as_c_str()) {
            return existing.as_ptr();
        }
        let boxed: Box<CStr> = c.into_boxed_c_str();
        let p = boxed.as_ptr();
        self.strings.insert(boxed);
        p
    }

    pub fn post(&mut self, callback: i32, data: Vec<u8>) {
        self.queue.push_back((callback, data));
    }

    pub fn new_call(&mut self) -> u64 {
        let id = self.next_call;
        self.next_call += 1;
        id
    }

    /// Finish an API call: store its result and announce it with
    /// SteamAPICallCompleted_t, which steam_api turns into the game's
    /// CCallResult.
    pub fn complete(&mut self, call: u64, callback: i32, data: Vec<u8>, failed: bool) {
        let mut done = Bytes::new(16);
        done.u64(0, call).i32(8, callback).u32(12, data.len() as u32);
        self.calls.insert(call, Call { callback, data, failed });
        self.post(cb::API_CALL_COMPLETED, done.0);
    }

    /// Start and immediately finish a call.
    pub fn call_now(&mut self, callback: i32, data: Vec<u8>) -> u64 {
        let id = self.new_call();
        self.complete(id, callback, data, false);
        id
    }

    pub fn app_id(&self) -> u32 {
        self.session.as_ref().map(|s| s.app_id).filter(|a| *a != 0).unwrap_or_else(env_app_id)
    }

    pub fn me(&self) -> u64 {
        self.session.as_ref().map(|s| s.steam_id).unwrap_or(0)
    }
}

pub fn env_app_id() -> u32 {
    std::env::var("SteamAppId").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// Talk to SteamInRust once: load who's signed in and the game's stats.
/// Returns whether a session exists. Never called with the lock held.
pub fn ensure_session() -> bool {
    {
        let g = lock();
        if g.session.is_some() {
            return true;
        }
        if g.hello_tried {
            return false;
        }
    }
    let reply = crate::ipc::request(Request::Hello);
    let mut g = lock();
    g.hello_tried = true;
    match reply {
        Some(Reply::Hello(s)) => {
            crate::log!("signed in as {} ({}), app {}, stats: {}", s.persona_name, s.steam_id, s.app_id, s.stats.as_ref().map(|x| x.stats.len()).unwrap_or(0));
            if let Some(stats) = &s.stats {
                g.stats.load(stats);
            }
            let (app, me, ok) = (s.app_id, s.steam_id, s.stats.is_some());
            g.session = Some(*s);
            crate::lobby::start_events();
            g.post(cb::USER_STATS_RECEIVED, user_stats_received(app, if ok { RESULT_OK } else { RESULT_FAIL }, me));
            true
        }
        other => {
            crate::log!("no session from SteamInRust: {other:?}");
            false
        }
    }
}

pub fn user_stats_received(app: u32, result: i32, user: u64) -> Vec<u8> {
    let mut b = Bytes::new(24);
    b.u64(0, app as u64).i32(8, result).u64(16, user);
    b.0
}

/// Little-endian struct builder with explicit offsets (callback structs use
/// Steam's 8-byte packing, so offsets are written out by hand).
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn new(size: usize) -> Self {
        Self(vec![0; size])
    }
    fn put(&mut self, at: usize, b: &[u8]) -> &mut Self {
        if let Some(dst) = self.0.get_mut(at..at + b.len()) {
            dst.copy_from_slice(b);
        }
        self
    }
    pub fn u64(&mut self, at: usize, v: u64) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    pub fn i32(&mut self, at: usize, v: i32) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    pub fn u32(&mut self, at: usize, v: u32) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    pub fn u8(&mut self, at: usize, v: u8) -> &mut Self {
        self.put(at, &[v])
    }
    pub fn str(&mut self, at: usize, max: usize, s: &str) -> &mut Self {
        let n = s.len().min(max.saturating_sub(1));
        self.put(at, &s.as_bytes()[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn achievement_bits_follow_blocks() {
        let mut s = StatsState::default();
        s.load(&Stats {
            crc: 7,
            stats: vec![Stat { id: 1, name: "NumGames".into(), kind: 1, value: 3, ..Default::default() }],
            achievements: vec![
                Achievement { name: "ACH_WIN_ONE_GAME".into(), block: 21, bit: 0, ..Default::default() },
                Achievement { name: "ACH_TRAVEL_FAR".into(), block: 21, bit: 2, unlocked: true, unlock_time: 5, ..Default::default() },
            ],
        });
        assert_eq!(s.stats.len(), 2, "block stat added");
        let i = s.ach("ach_win_one_game").unwrap();
        s.set_ach(i, true);
        let block = s.stats.iter().find(|x| x.def.id == 21).unwrap();
        assert_eq!(block.value, 0b101, "bit 2 from the loaded unlock, bit 0 just set");
        assert_eq!(block.stored, 0b100);
        assert!(s.stat("NumGames").is_some());
        assert!(s.stat("").is_none(), "block stats aren't visible by name");
    }

    #[test]
    fn callback_bytes() {
        let v = user_stats_received(480, 1, 76561197960287930);
        assert_eq!(v.len(), 24);
        assert_eq!(u64::from_le_bytes(v[16..24].try_into().unwrap()), 76561197960287930);
    }
}

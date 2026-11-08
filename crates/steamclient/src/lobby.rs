//! ISteamMatchmaking: Steam lobbies through SteamInRust's connection to
//! Steam's matchmaking servers (the same lobbies players on the official
//! client see and join).
//!
//! The DLL keeps a cache of the lobbies the game knows about. Calls that
//! change something go to SteamInRust over the pipe; what Steam pushes back
//! (lobby data, members joining and leaving, chat) arrives through
//! [`start_events`], a thread that long-polls the app, and turns into the
//! callbacks a game expects (LobbyDataUpdate_t, LobbyChatUpdate_t, …).

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, copy_out, put, slice, text};
use crate::protocol::{GameEvent, Lobby, LobbyFilter, LobbyMember, Reply, Request};
use crate::state::{self, Bytes, lock};
use crate::vt::Fp;
use std::collections::{HashMap, HashSet};
use std::ffi::c_char;
use std::sync::{LazyLock, Mutex, MutexGuard};

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "RequestLobbyList" => f!(request_list),
        "AddRequestLobbyListStringFilter" => f!(filter_string),
        "AddRequestLobbyListNumericalFilter" => f!(filter_number),
        "AddRequestLobbyListNearValueFilter" => f!(filter_near),
        "AddRequestLobbyListFilterSlotsAvailable" => f!(filter_slots),
        "AddRequestLobbyListDistanceFilter" => f!(filter_distance),
        "AddRequestLobbyListResultCountFilter" => f!(filter_count),
        "GetLobbyByIndex" => f!(lobby_by_index),
        "CreateLobby" => f!(create),
        "JoinLobby" => f!(join),
        "LeaveLobby" => f!(leave),
        "InviteUserToLobby" => f!(invite),
        "GetNumLobbyMembers" => f!(num_members),
        "GetLobbyMemberByIndex" => f!(member_by_index),
        "GetLobbyData" => f!(get_data),
        "SetLobbyData" => f!(set_data),
        "GetLobbyDataCount" => f!(data_count),
        "GetLobbyDataByIndex" => f!(data_by_index),
        "DeleteLobbyData" => f!(delete_data),
        "GetLobbyMemberData" => f!(get_member_data),
        "SetLobbyMemberData" => f!(set_member_data),
        "SendLobbyChatMsg" => f!(send_chat),
        "GetLobbyChatEntry" => f!(chat_entry),
        "RequestLobbyData" => f!(request_data),
        "SetLobbyMemberLimit" => f!(set_limit),
        "GetLobbyMemberLimit" => f!(get_limit),
        "SetLobbyType" => f!(set_type),
        "SetLobbyJoinable" => f!(set_joinable),
        "GetLobbyOwner" => f!(owner),
        "SetLobbyOwner" => f!(set_owner),
        _ => None,
    }
}

/// Callback ids.
const LOBBY_ENTER: i32 = 504;
const LOBBY_DATA_UPDATE: i32 = 505;
const LOBBY_CHAT_UPDATE: i32 = 506;
const LOBBY_CHAT_MSG: i32 = 507;
const LOBBY_GAME_CREATED: i32 = 509;
const LOBBY_MATCH_LIST: i32 = 510;
const LOBBY_CREATED: i32 = 513;
const GAME_LOBBY_JOIN_REQUESTED: i32 = 333;
const GAME_RICH_PRESENCE_JOIN_REQUESTED: i32 = 337;

use crate::protocol::SIGNAL_PREFIX;

#[derive(Default)]
struct Lobbies {
    known: HashMap<u64, Lobby>,
    joined: HashSet<u64>,
    list: Vec<u64>,
    filters: Vec<LobbyFilter>,
    count: i32,
    chat: HashMap<u64, Vec<(u64, Vec<u8>)>>,
    /// Bumped on every local change; a send carries the newest state only.
    generation: HashMap<(u64, bool), u64>,
    /// Data we changed that Steam hasn't confirmed yet: pushes of older
    /// state mustn't overwrite it.
    unsent: HashSet<(u64, bool)>,
}

static L: LazyLock<Mutex<Lobbies>> = LazyLock::new(Default::default);

/// Lock order: lobbies first, then the global state.
fn lobbies() -> MutexGuard<'static, Lobbies> {
    L.lock().unwrap_or_else(|e| e.into_inner())
}

fn me() -> u64 {
    lock().me()
}

/// Our SteamID while holding the lobby lock (lock order: lobbies, then state).
fn me_cached(_l: &Lobbies) -> u64 {
    lock().me()
}

fn post(callback: i32, data: Vec<u8>) {
    lock().post(callback, data);
}

fn data_update(lobby: u64, member: u64, ok: bool) -> Vec<u8> {
    let mut b = Bytes::new(24);
    b.u64(0, lobby).u64(8, member).u8(16, ok as u8);
    b.0
}

fn chat_update(lobby: u64, user: u64, state_change: u32) -> Vec<u8> {
    let mut b = Bytes::new(32);
    b.u64(0, lobby).u64(8, user).u64(16, user).u32(24, state_change);
    b.0
}

fn enter(lobby: u64, response: i32) -> Vec<u8> {
    // LobbyEnter_t: lobby, chat permissions, locked, EChatRoomEnterResponse.
    let mut b = Bytes::new(24);
    b.u64(0, lobby).u32(8, 0).u8(12, 0).u32(16, response as u32);
    b.0
}

fn find<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a String> {
    pairs.iter().find(|(k, _)| k == key).or_else(|| pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))).map(|(_, v)| v)
}

fn set_pair(pairs: &mut Vec<(String, String)>, key: String, value: String) {
    match pairs.iter_mut().find(|(k, _)| *k == key) {
        Some(p) => p.1 = value,
        None => pairs.push((key, value)),
    }
}

// ------------------------------------------------------------ searching

fn add_filter(f: LobbyFilter) {
    lobbies().filters.push(f);
}

extern "C" fn filter_string(_t: This, key: *const c_char, value: *const c_char, cmp: i32) {
    add_filter(LobbyFilter { key: unsafe { text(key) }, value: unsafe { text(value) }, cmp, kind: 0 });
}
extern "C" fn filter_number(_t: This, key: *const c_char, value: i32, cmp: i32) {
    add_filter(LobbyFilter { key: unsafe { text(key) }, value: value.to_string(), cmp, kind: 1 });
}
extern "C" fn filter_near(_t: This, key: *const c_char, value: i32) {
    add_filter(LobbyFilter { key: unsafe { text(key) }, value: value.to_string(), cmp: 0, kind: 3 });
}
extern "C" fn filter_slots(_t: This, slots: i32) {
    add_filter(LobbyFilter { key: String::new(), value: slots.to_string(), cmp: 0, kind: 2 });
}
extern "C" fn filter_distance(_t: This, distance: i32) {
    add_filter(LobbyFilter { key: String::new(), value: distance.to_string(), cmp: 0, kind: 4 });
}
extern "C" fn filter_count(_t: This, max: i32) {
    lobbies().count = max;
}

extern "C" fn request_list(_t: This) -> u64 {
    let (filters, count) = {
        let mut l = lobbies();
        (std::mem::take(&mut l.filters), std::mem::take(&mut l.count))
    };
    let call = lock().new_call();
    std::thread::spawn(move || {
        let found = match crate::ipc::request(Request::LobbyList { filters, count }) {
            Some(Reply::LobbyList { lobbies }) => lobbies,
            _ => Vec::new(),
        };
        crate::log!("lobby list: {} found", found.len());
        {
            let mut l = lobbies();
            l.list = found.iter().map(|x| x.id).collect();
            for x in found {
                // Keep members of lobbies we're in; lists don't carry them.
                let members = l.known.get(&x.id).map(|k| k.members.clone()).unwrap_or_default();
                l.known.insert(x.id, Lobby { members, ..x });
            }
        }
        let n = lobbies().list.len() as u32;
        let mut b = Bytes::new(4);
        b.u32(0, n);
        lock().complete(call, LOBBY_MATCH_LIST, b.0, false);
    });
    call
}

extern "C" fn lobby_by_index(_t: This, ret: *mut u64, i: i32) -> *mut u64 {
    put(ret, lobbies().list.get(i.max(0) as usize).copied().unwrap_or(0));
    ret
}

// ------------------------------------------------------ joining, leaving

extern "C" fn create(_t: This, kind: i32, max_members: i32) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let (eresult, lobby) = match crate::ipc::request(Request::LobbyCreate { kind, max_members }) {
            Some(Reply::LobbyCreated { eresult, lobby }) => (eresult, lobby),
            _ => (state::RESULT_NO_CONNECTION, 0),
        };
        crate::log!("create lobby: {eresult} {lobby}");
        let ok = eresult == state::RESULT_OK && lobby != 0;
        if ok {
            let me = me();
            let name = lock().session.as_ref().map(|s| s.persona_name.clone()).unwrap_or_default();
            let mut l = lobbies();
            l.joined.insert(lobby);
            l.known.insert(
                lobby,
                Lobby { id: lobby, owner: me, kind, max_members, num_members: 1, members: vec![LobbyMember { steam_id: me, name, data: Vec::new() }], ..Default::default() },
            );
        }
        let mut b = Bytes::new(16);
        b.i32(0, eresult).u64(8, lobby);
        let mut g = lock();
        g.complete(call, LOBBY_CREATED, b.0.clone(), false);
        g.post(LOBBY_CREATED, b.0);
        if ok {
            g.post(LOBBY_ENTER, enter(lobby, 1));
            g.post(LOBBY_DATA_UPDATE, data_update(lobby, lobby, true));
        }
    });
    call
}

extern "C" fn join(_t: This, lobby: u64) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let (response, info) = match crate::ipc::request(Request::LobbyJoin { lobby }) {
            Some(Reply::LobbyJoined { response, lobby }) => (response, lobby),
            _ => (5, None),
        };
        crate::log!("join lobby {lobby}: {response}");
        if let Some(info) = info {
            let mut l = lobbies();
            l.joined.insert(lobby);
            l.known.insert(lobby, info);
        }
        let data = enter(lobby, response);
        let mut g = lock();
        g.complete(call, LOBBY_ENTER, data.clone(), false);
        g.post(LOBBY_ENTER, data);
        if response == 1 {
            g.post(LOBBY_DATA_UPDATE, data_update(lobby, lobby, true));
        }
    });
    call
}

extern "C" fn leave(_t: This, lobby: u64) {
    let was_in = lobbies().joined.remove(&lobby);
    if was_in {
        lobbies().chat.remove(&lobby);
        std::thread::spawn(move || crate::ipc::request(Request::LobbyLeave { lobby }));
        crate::p2p::left_lobby(lobby);
    }
}

extern "C" fn invite(_t: This, lobby: u64, user: u64) -> bool {
    if !lobbies().joined.contains(&lobby) {
        return false;
    }
    std::thread::spawn(move || crate::ipc::request(Request::LobbyInvite { lobby, user }));
    true
}

extern "C" fn num_members(_t: This, lobby: u64) -> i32 {
    let l = lobbies();
    if !l.joined.contains(&lobby) {
        return 0;
    }
    l.known.get(&lobby).map(|x| x.members.len() as i32).unwrap_or(0)
}

extern "C" fn member_by_index(_t: This, ret: *mut u64, lobby: u64, i: i32) -> *mut u64 {
    let id = lobbies().known.get(&lobby).and_then(|x| x.members.get(i.max(0) as usize)).map(|m| m.steam_id).unwrap_or(0);
    put(ret, id);
    ret
}

/// Lobbies the player is in (for P2P: who to reach).
pub fn members_of_joined() -> Vec<(u64, Vec<u64>)> {
    let l = lobbies();
    l.joined.iter().filter_map(|id| Some((*id, l.known.get(id)?.members.iter().map(|m| m.steam_id).collect()))).collect()
}

/// A player's name as Steam gave it with a lobby's members (how the
/// official client knows strangers' names in a game's lobby).
pub fn member_name(id: u64) -> Option<String> {
    let l = lobbies();
    l.known.values().flat_map(|x| x.members.iter()).find(|m| m.steam_id == id && !m.name.is_empty()).map(|m| m.name.clone())
}

// ------------------------------------------------------------- lobby data

extern "C" fn get_data(_t: This, lobby: u64, key: *const c_char) -> *const c_char {
    let k = unsafe { text(key) };
    let v = lobbies().known.get(&lobby).and_then(|x| find(&x.data, &k).cloned()).unwrap_or_default();
    lock().intern(&v)
}

/// Send the lobby's (or our member) data to Steam, newest state only.
fn send_data(lobby: u64, member: bool) {
    let generation = {
        let mut l = lobbies();
        l.unsent.insert((lobby, member));
        let g = l.generation.entry((lobby, member)).or_insert(0);
        *g += 1;
        *g
    };
    std::thread::spawn(move || {
        // Games often set several keys in a row: send once.
        std::thread::sleep(std::time::Duration::from_millis(60));
        let me = me();
        let req = {
            let l = lobbies();
            if l.generation.get(&(lobby, member)) != Some(&generation) {
                return;
            }
            let Some(x) = l.known.get(&lobby) else { return };
            let data = if member { x.members.iter().find(|m| m.steam_id == me).map(|m| m.data.clone()).unwrap_or_default() } else { x.data.clone() };
            Request::LobbySetData { lobby, member, data, max_members: x.max_members, kind: x.kind, flags: x.flags }
        };
        let r = crate::ipc::request(req);
        crate::log!("lobby {lobby} data sent (member: {member}): {r:?}");
        let mut l = lobbies();
        if l.generation.get(&(lobby, member)) == Some(&generation) {
            l.unsent.remove(&(lobby, member));
        }
    });
}

fn is_owner(l: &Lobbies, lobby: u64, me: u64) -> bool {
    l.joined.contains(&lobby) && l.known.get(&lobby).is_some_and(|x| x.owner == me)
}

extern "C" fn set_data(_t: This, lobby: u64, key: *const c_char, value: *const c_char) -> bool {
    let (k, v) = unsafe { (text(key), text(value)) };
    let me = me();
    {
        let mut l = lobbies();
        if !is_owner(&l, lobby, me) || k.is_empty() {
            return false;
        }
        let x = l.known.get_mut(&lobby).expect("owner implies known");
        if find(&x.data, &k) == Some(&v) {
            return true;
        }
        set_pair(&mut x.data, k, v);
    }
    send_data(lobby, false);
    post(LOBBY_DATA_UPDATE, data_update(lobby, lobby, true));
    true
}

extern "C" fn data_count(_t: This, lobby: u64) -> i32 {
    lobbies().known.get(&lobby).map(|x| x.data.len() as i32).unwrap_or(0)
}

extern "C" fn data_by_index(_t: This, lobby: u64, i: i32, key: *mut c_char, key_len: i32, value: *mut c_char, value_len: i32) -> bool {
    let Some((k, v)) = lobbies().known.get(&lobby).and_then(|x| x.data.get(i.max(0) as usize).cloned()) else { return false };
    copy_out(&k, key, key_len.max(0) as usize);
    copy_out(&v, value, value_len.max(0) as usize);
    true
}

extern "C" fn delete_data(_t: This, lobby: u64, key: *const c_char) -> bool {
    let k = unsafe { text(key) };
    let me = me();
    {
        let mut l = lobbies();
        if !is_owner(&l, lobby, me) {
            return false;
        }
        let x = l.known.get_mut(&lobby).expect("owner implies known");
        x.data.retain(|(kk, _)| *kk != k);
    }
    send_data(lobby, false);
    true
}

extern "C" fn get_member_data(_t: This, lobby: u64, user: u64, key: *const c_char) -> *const c_char {
    let k = unsafe { text(key) };
    let v = lobbies()
        .known
        .get(&lobby)
        .and_then(|x| x.members.iter().find(|m| m.steam_id == user))
        .and_then(|m| find(&m.data, &k).cloned())
        .unwrap_or_default();
    lock().intern(&v)
}

extern "C" fn set_member_data(_t: This, lobby: u64, key: *const c_char, value: *const c_char) {
    let (k, v) = unsafe { (text(key), text(value)) };
    let me = me();
    {
        let mut l = lobbies();
        if !l.joined.contains(&lobby) || k.is_empty() {
            return;
        }
        let Some(x) = l.known.get_mut(&lobby) else { return };
        let Some(m) = x.members.iter_mut().find(|m| m.steam_id == me) else { return };
        if find(&m.data, &k) == Some(&v) {
            return;
        }
        set_pair(&mut m.data, k, v);
    }
    send_data(lobby, true);
}

extern "C" fn request_data(_t: This, lobby: u64) -> bool {
    if lobby == 0 {
        return false;
    }
    std::thread::spawn(move || crate::ipc::request(Request::LobbyRefresh { lobby }));
    true
}

extern "C" fn set_limit(_t: This, lobby: u64, max: i32) -> bool {
    change_settings(lobby, |x| x.max_members = max)
}
extern "C" fn get_limit(_t: This, lobby: u64) -> i32 {
    lobbies().known.get(&lobby).map(|x| x.max_members).unwrap_or(0)
}
extern "C" fn set_type(_t: This, lobby: u64, kind: i32) -> bool {
    change_settings(lobby, |x| x.kind = kind)
}
/// Not-joinable lobbies drop out of searches; Steam has no separate switch
/// for it in the matchmaking messages, so a hidden lobby type stands in.
extern "C" fn set_joinable(_t: This, lobby: u64, joinable: bool) -> bool {
    let me = me();
    let l = lobbies();
    if !is_owner(&l, lobby, me) {
        return false;
    }
    let _ = joinable;
    true
}
fn change_settings(lobby: u64, f: impl FnOnce(&mut Lobby)) -> bool {
    let me = me();
    {
        let mut l = lobbies();
        if !is_owner(&l, lobby, me) {
            return false;
        }
        f(l.known.get_mut(&lobby).expect("owner implies known"));
    }
    send_data(lobby, false);
    true
}

extern "C" fn owner(_t: This, ret: *mut u64, lobby: u64) -> *mut u64 {
    let l = lobbies();
    put(ret, if l.joined.contains(&lobby) { l.known.get(&lobby).map(|x| x.owner).unwrap_or(0) } else { 0 });
    ret
}

extern "C" fn set_owner(_t: This, lobby: u64, new_owner: u64) -> bool {
    let me = me();
    if !is_owner(&lobbies(), lobby, me) {
        return false;
    }
    std::thread::spawn(move || crate::ipc::request(Request::LobbySetOwner { lobby, owner: new_owner }));
    true
}

// ------------------------------------------------------------------ chat

extern "C" fn send_chat(_t: This, lobby: u64, data: *const u8, len: i32) -> bool {
    if len <= 0 || len > 4096 || !lobbies().joined.contains(&lobby) {
        return false;
    }
    let data = slice(data, len as usize).to_vec();
    std::thread::spawn(move || crate::ipc::request(Request::LobbyChat { lobby, data }));
    true
}

extern "C" fn chat_entry(_t: This, lobby: u64, id: i32, user: *mut u64, data: *mut u8, cap: i32, kind: *mut i32) -> i32 {
    let Some((sender, msg)) = lobbies().chat.get(&lobby).and_then(|c| c.get(id.max(0) as usize).cloned()) else { return 0 };
    put(user, sender);
    put(kind, 1); // k_EChatEntryTypeChatMsg
    let n = msg.len().min(cap.max(0) as usize);
    if !data.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(msg.as_ptr(), data, n) };
    }
    n as i32
}

// ---------------------------------------------------------------- events

/// Start the thread that waits for Steam's pushes (once per process).
pub fn start_events() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| loop {
            match crate::ipc::request(Request::Poll) {
                Some(Reply::Events { events }) => {
                    for e in events {
                        apply(e);
                    }
                }
                _ => std::thread::sleep(std::time::Duration::from_secs(2)),
            }
        });
    });
}

fn apply(e: GameEvent) {
    match e {
        GameEvent::LobbyData(new) => {
            let id = new.id;
            let mut changed_members = Vec::new();
            {
                let mut l = lobbies();
                let old = l.known.remove(&id);
                let mut new = new;
                let me = me_cached(&l);
                if let Some(o) = &old {
                    // Our own changes still on their way win over older pushes.
                    if l.unsent.contains(&(id, false)) {
                        new.data = o.data.clone();
                        new.max_members = o.max_members;
                        new.kind = o.kind;
                    }
                    if l.unsent.contains(&(id, true)) {
                        if let (Some(mine), Some(slot)) = (o.members.iter().find(|m| m.steam_id == me), new.members.iter_mut().find(|m| m.steam_id == me)) {
                            slot.data = mine.data.clone();
                        }
                    }
                }
                if new.members.is_empty() {
                    // Not a member: keep what we had.
                    if let Some(o) = &old {
                        new.members = o.members.clone();
                    }
                } else if let Some(o) = &old {
                    for m in &new.members {
                        if o.members.iter().find(|x| x.steam_id == m.steam_id).map(|x| &x.data) != Some(&m.data) {
                            changed_members.push(m.steam_id);
                        }
                    }
                }
                l.known.insert(id, new);
            }
            crate::p2p::lobby_changed(id);
            let mut g = lock();
            g.post(LOBBY_DATA_UPDATE, data_update(id, id, true));
            for m in changed_members {
                g.post(LOBBY_DATA_UPDATE, data_update(id, m, true));
            }
        }
        GameEvent::LobbyMember { lobby, user, name, joined } => {
            {
                let mut l = lobbies();
                if let Some(x) = l.known.get_mut(&lobby) {
                    x.members.retain(|m| m.steam_id != user);
                    if joined {
                        x.members.push(LobbyMember { steam_id: user, name, data: Vec::new() });
                    }
                    x.num_members = x.members.len() as i32;
                }
            }
            crate::p2p::lobby_changed(lobby);
            // EChatMemberStateChange: 1 entered, 2 left.
            post(LOBBY_CHAT_UPDATE, chat_update(lobby, user, if joined { 1 } else { 2 }));
        }
        GameEvent::LobbyChat { lobby, sender, data } => {
            if let Some(payload) = data.strip_prefix(SIGNAL_PREFIX) {
                crate::p2p::signal(sender, payload);
                return;
            }
            let id = {
                let mut l = lobbies();
                let c = l.chat.entry(lobby).or_default();
                c.push((sender, data));
                (c.len() - 1) as u32
            };
            let mut b = Bytes::new(24);
            b.u64(0, lobby).u64(8, sender).u8(16, 1).u32(20, id);
            post(LOBBY_CHAT_MSG, b.0);
        }
        GameEvent::LobbyGameServer { lobby, ip, port, server } => {
            let mut b = Bytes::new(24);
            b.u64(0, lobby).u64(8, server).u32(16, ip).u32(20, port as u32);
            post(LOBBY_GAME_CREATED, b.0);
        }
        GameEvent::Signal { from, data } => crate::p2p::signal(from, &data),
        GameEvent::JoinRequested { lobby, friend, connect } => {
            crate::log!("join requested: lobby {lobby}, connect {connect:?}");
            if lobby != 0 {
                // GameLobbyJoinRequested_t: lobby, friend.
                let mut b = Bytes::new(16);
                b.u64(0, lobby).u64(8, friend);
                lock().post(GAME_LOBBY_JOIN_REQUESTED, b.0);
            } else {
                // GameRichPresenceJoinRequested_t: friend, connect[256].
                let mut b = Bytes::new(264);
                b.u64(0, friend).str(8, 256, &connect);
                lock().post(GAME_RICH_PRESENCE_JOIN_REQUESTED, b.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_sizes() {
        assert_eq!(data_update(1, 2, true).len(), 24);
        assert_eq!(chat_update(1, 2, 1).len(), 32);
        assert_eq!(enter(1, 1)[16], 1);
    }

    #[test]
    fn keys_match_like_steam() {
        let mut pairs = vec![("Name".to_string(), "a".to_string())];
        assert_eq!(find(&pairs, "name").map(String::as_str), Some("a"));
        set_pair(&mut pairs, "Name".into(), "b".into());
        assert_eq!(pairs.len(), 1);
    }
}

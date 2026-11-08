//! What the game gets when it calls a Steam interface method.
//!
//! Each function matches the C++ method it stands in for: `this` first, then
//! the method's arguments; methods returning a class take a hidden return
//! pointer right after `this`. Data comes from the session SteamInRust sent
//! at start-up; slow things (storing stats, other players' stats) run on a
//! thread and finish as Steam API call results.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::protocol::{Reply, Request, Session};
use crate::state::{self, Bytes, Global, cb, lock};
use crate::vt::Fp;
use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;

pub(crate) type This = *mut c_void;

/// Our implementation of `kind::name`, if we have one.
pub fn imp(kind: &str, name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    if let Some(f) = crate::extras::imp(kind, name).or_else(|| crate::sns::imp(kind, name)) {
        return Some(f);
    }
    match (kind, name) {
        // ISteamClient
        ("ISteamClient", "CreateSteamPipe") => f!(c_create_pipe),
        ("ISteamClient", "BReleaseSteamPipe") => f!(c_release_pipe),
        ("ISteamClient", "ConnectToGlobalUser") => f!(c_connect_global),
        ("ISteamClient", "CreateLocalUser") => f!(c_create_local),
        ("ISteamClient", "ReleaseUser") => f!(c_release_user),
        ("ISteamClient", "GetISteamUtils") => f!(c_get_utils),
        ("ISteamClient", "BShutdownIfAllPipesClosed") => f!(c_shutdown_if_closed),
        ("ISteamClient", m) if m.starts_with("GetISteam") => f!(c_get),

        // ISteamUser
        ("ISteamUser", "GetHSteamUser") => f!(one),
        ("ISteamUser", "BLoggedOn") => f!(logged_on),
        ("ISteamUser", "GetSteamID") => f!(u_steam_id),
        ("ISteamUser", "GetUserDataFolder") => f!(u_userdata_folder),
        ("ISteamUser", "GetPlayerSteamLevel") => f!(u_level),
        ("ISteamUser", "UserHasLicenseForApp") => f!(u_has_license),
        ("ISteamUser", "BIsPhoneVerified" | "BIsTwoFactorEnabled") => f!(yes),
        ("ISteamUser", "GetAuthSessionTicket") => f!(u_auth_ticket),
        ("ISteamUser", "GetAuthTicketForWebApi") => f!(u_web_api_ticket),
        ("ISteamUser", "CancelAuthTicket") => f!(u_cancel_ticket),
        ("ISteamUser", "BeginAuthSession") => f!(u_begin_auth),
        ("ISteamUser", "EndAuthSession") => f!(u_end_auth),
        ("ISteamUser", "RequestEncryptedAppTicket") => f!(u_request_encrypted_ticket),
        ("ISteamUser", "GetEncryptedAppTicket") => f!(u_encrypted_ticket),

        // ISteamUtils
        ("ISteamUtils", "GetSecondsSinceAppActive" | "GetSecondsSinceComputerActive") => f!(ut_seconds),
        ("ISteamUtils", "GetConnectedUniverse") => f!(one),
        ("ISteamUtils", "GetServerRealTime") => f!(ut_real_time),
        ("ISteamUtils", "GetIPCountry") => f!(ut_country),
        ("ISteamUtils", "GetCurrentBatteryPower") => f!(ut_battery),
        ("ISteamUtils", "GetAppID") => f!(ut_app_id),
        ("ISteamUtils", "IsAPICallCompleted") => f!(ut_call_completed),
        ("ISteamUtils", "GetAPICallFailureReason") => f!(ut_call_failure),
        ("ISteamUtils", "GetAPICallResult") => f!(ut_call_result),
        ("ISteamUtils", "GetSteamUILanguage") => f!(ut_ui_language),
        ("ISteamUtils", "InitFilterText") => f!(yes),
        ("ISteamUtils", "FilterText") => f!(ut_filter_text),

        // ISteamApps
        ("ISteamApps", "BIsSubscribed") => f!(has_session),
        ("ISteamApps", "GetCurrentGameLanguage" | "GetAvailableGameLanguages") => f!(a_game_language),
        ("ISteamApps", "BIsSubscribedApp") => f!(a_owns),
        ("ISteamApps", "BIsDlcInstalled") => f!(a_has_dlc),
        ("ISteamApps", "GetDLCCount") => f!(a_dlc_count),
        ("ISteamApps", "BGetDLCDataByIndex") => f!(a_dlc_data),
        ("ISteamApps", "GetCurrentBetaName") => f!(a_beta_name),
        ("ISteamApps", "GetAppInstallDir") => f!(a_install_dir),
        ("ISteamApps", "BIsAppInstalled") => f!(a_installed),
        ("ISteamApps", "GetAppOwner") => f!(u_steam_id),
        ("ISteamApps", "GetLaunchQueryParam") => f!(empty_str),
        ("ISteamApps", "GetAppBuildId") => f!(a_build_id),
        ("ISteamApps", "SetDlcContext") => f!(yes),

        // ISteamFriends
        ("ISteamFriends", "GetPersonaName") => f!(fr_persona_name),
        ("ISteamFriends", "GetPersonaState") => f!(one),
        ("ISteamFriends", "GetFriendCount") => f!(fr_count),
        ("ISteamFriends", "GetFriendByIndex") => f!(fr_by_index),
        ("ISteamFriends", "GetFriendRelationship") => f!(fr_relationship),
        ("ISteamFriends", "GetFriendPersonaState") => f!(fr_state),
        ("ISteamFriends", "GetFriendPersonaName") => f!(fr_name),
        ("ISteamFriends", "GetFriendGamePlayed") => f!(fr_game_played),
        ("ISteamFriends", "GetFriendPersonaNameHistory" | "GetClanName" | "GetClanTag" | "GetFriendsGroupName") => f!(empty_str),
        ("ISteamFriends", "GetFriendSteamLevel") => f!(fr_level),
        ("ISteamFriends", "HasFriend") => f!(fr_has_friend),
        ("ISteamFriends", "ActivateGameOverlayToWebPage") => f!(fr_overlay_web),
        ("ISteamFriends", "ActivateGameOverlayToStore") => f!(fr_overlay_store),
        ("ISteamFriends", "SetRichPresence") => f!(fr_set_rich_presence),
        ("ISteamFriends", "ClearRichPresence") => f!(fr_clear_rich_presence),
        ("ISteamFriends", "GetFriendRichPresence") => f!(fr_rich_presence),
        ("ISteamFriends", "GetFriendRichPresenceKeyCount") => f!(fr_rich_presence_count),
        ("ISteamFriends", "GetFriendRichPresenceKeyByIndex") => f!(fr_rich_presence_key),

        // ISteamUserStats
        ("ISteamUserStats", "RequestCurrentStats") => f!(st_request_current),
        ("ISteamUserStats", "GetStatInt32") => f!(st_get_int),
        ("ISteamUserStats", "GetStatFloat") => f!(st_get_float),
        ("ISteamUserStats", "SetStatInt32") => f!(st_set_int),
        ("ISteamUserStats", "SetStatFloat") => f!(st_set_float),
        ("ISteamUserStats", "UpdateAvgRateStat") => f!(st_update_avg),
        ("ISteamUserStats", "GetAchievement") => f!(st_get_ach),
        ("ISteamUserStats", "SetAchievement") => f!(st_set_ach),
        ("ISteamUserStats", "ClearAchievement") => f!(st_clear_ach),
        ("ISteamUserStats", "GetAchievementAndUnlockTime") => f!(st_get_ach_time),
        ("ISteamUserStats", "StoreStats") => f!(st_store),
        ("ISteamUserStats", "GetAchievementDisplayAttribute") => f!(st_ach_attr),
        ("ISteamUserStats", "IndicateAchievementProgress") => f!(st_indicate_progress),
        ("ISteamUserStats", "GetNumAchievements") => f!(st_num_achs),
        ("ISteamUserStats", "GetAchievementName") => f!(st_ach_name),
        ("ISteamUserStats", "RequestUserStats") => f!(st_request_user),
        ("ISteamUserStats", "GetUserStatInt32") => f!(st_user_int),
        ("ISteamUserStats", "GetUserStatFloat") => f!(st_user_float),
        ("ISteamUserStats", "GetUserAchievement") => f!(st_user_ach),
        ("ISteamUserStats", "GetUserAchievementAndUnlockTime") => f!(st_user_ach_time),
        ("ISteamUserStats", "ResetAllStats") => f!(st_reset),
        ("ISteamUserStats", "AttachLeaderboardUGC") => f!(st_attach_ugc),
        ("ISteamUserStats", "GetNumberOfCurrentPlayers") => f!(st_current_players),
        ("ISteamUserStats", "RequestGlobalStats") => f!(st_global_stats),

        // ISteamRemoteStorage (the Steam Cloud file API)
        ("ISteamRemoteStorage", "FileWrite") => f!(rs_write),
        ("ISteamRemoteStorage", "FileRead") => f!(rs_read),
        ("ISteamRemoteStorage", "FileWriteAsync") => f!(rs_write_async),
        ("ISteamRemoteStorage", "FileReadAsync") => f!(rs_read_async),
        ("ISteamRemoteStorage", "FileReadAsyncComplete") => f!(rs_read_async_complete),
        ("ISteamRemoteStorage", "FileForget" | "SetSyncPlatforms" | "IsCloudEnabledForAccount" | "IsCloudEnabledForApp") => f!(yes),
        ("ISteamRemoteStorage", "BeginFileWriteBatch" | "EndFileWriteBatch") => f!(yes),
        ("ISteamRemoteStorage", "FileDelete") => f!(rs_delete),
        ("ISteamRemoteStorage", "FileShare") => f!(rs_share),
        ("ISteamRemoteStorage", "FileWriteStreamOpen") => f!(rs_stream_open),
        ("ISteamRemoteStorage", "FileWriteStreamWriteChunk") => f!(rs_stream_write),
        ("ISteamRemoteStorage", "FileWriteStreamClose") => f!(rs_stream_close),
        ("ISteamRemoteStorage", "FileWriteStreamCancel") => f!(rs_stream_cancel),
        ("ISteamRemoteStorage", "FileExists" | "FilePersisted") => f!(rs_exists),
        ("ISteamRemoteStorage", "GetFileSize") => f!(rs_size),
        ("ISteamRemoteStorage", "GetFileTimestamp") => f!(rs_timestamp),
        ("ISteamRemoteStorage", "GetSyncPlatforms") => f!(all_platforms),
        ("ISteamRemoteStorage", "GetFileCount") => f!(rs_count),
        ("ISteamRemoteStorage", "GetFileNameAndSize") => f!(rs_name_and_size),
        ("ISteamRemoteStorage", "GetQuota") => f!(rs_quota),
        ("ISteamRemoteStorage", "GetLocalFileChange") => f!(empty_str),

        // ISteamMatchmaking: Steam lobbies (lobby.rs); P2P packets (p2p.rs).
        ("ISteamMatchmaking", m) => crate::lobby::imp(m),
        ("ISteamNetworking", m) => crate::p2p::imp(m),

        // ISteamNetworkingUtils
        ("ISteamNetworkingUtils", "GetLocalTimestamp") => f!(nu_timestamp),
        ("ISteamNetworkingUtils", "GetConfigValue") => f!(minus_one),
        _ => None,
    }
}

// ---------------------------------------------------------------- helpers

pub(crate) unsafe fn text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// Copy `s` plus a NUL into `buf` (`len` bytes). Returns bytes written.
pub(crate) fn copy_out(s: &str, buf: *mut c_char, len: usize) -> usize {
    if buf.is_null() || len == 0 {
        return 0;
    }
    let n = s.len().min(len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), n);
        *buf.add(n) = 0;
    }
    n + 1
}

pub(crate) fn put<T>(p: *mut T, v: T) {
    if !p.is_null() {
        unsafe { p.write_unaligned(v) };
    }
}

fn with_session<R>(f: impl FnOnce(&Session) -> R) -> Option<R> {
    lock().session.as_ref().map(f)
}

extern "C" fn one() -> i32 {
    1
}
extern "C" fn yes() -> bool {
    true
}
extern "C" fn minus_one() -> i32 {
    -1
}
extern "C" fn all_platforms() -> u32 {
    u32::MAX
}
extern "C" fn empty_str() -> *const c_char {
    c"".as_ptr()
}
/// Signed in and connected; false in offline mode, like Steam.
extern "C" fn logged_on(_t: This) -> bool {
    lock().session.as_ref().is_some_and(|s| !s.offline)
}
/// A SteamInRust session exists (online or offline).
extern "C" fn has_session(_t: This) -> bool {
    lock().session.is_some()
}

// ----------------------------------------------------------- ISteamClient

extern "C" fn c_create_pipe(_t: This) -> i32 {
    1
}
extern "C" fn c_release_pipe(_t: This, _pipe: i32) -> bool {
    true
}
extern "C" fn c_connect_global(_t: This, _pipe: i32) -> i32 {
    if state::ensure_session() { 1 } else { 0 }
}
/// Used by game servers (SteamGameServer_Init); they get a user that can't
/// do much, like an anonymous server.
extern "C" fn c_create_local(_t: This, pipe: *mut i32, _account_type: i32) -> i32 {
    put(pipe, 1);
    1
}
extern "C" fn c_release_user(_t: This, _pipe: i32, _user: i32) {}
extern "C" fn c_shutdown_if_closed(_t: This) -> bool {
    false
}
/// Every `GetISteamXxx(user, pipe, version)`: the version picks the object.
extern "C" fn c_get(_t: This, _user: i32, _pipe: i32, version: *const c_char) -> *mut c_void {
    interface(version)
}
extern "C" fn c_get_utils(_t: This, _pipe: i32, version: *const c_char) -> *mut c_void {
    interface(version)
}

pub fn interface(version: *const c_char) -> *mut c_void {
    let v = unsafe { text(version) };
    if v.is_empty() { std::ptr::null_mut() } else { crate::vt::get(&v) }
}

// ------------------------------------------------------------- ISteamUser

extern "C" fn u_steam_id(_t: This, ret: *mut u64) -> *mut u64 {
    put(ret, lock().me());
    ret
}
extern "C" fn u_userdata_folder(_t: This, buf: *mut c_char, len: i32) -> bool {
    let Some(dir) = user_dir().map(|d| d.join("local")) else { return false };
    let _ = std::fs::create_dir_all(&dir);
    let s = dir.to_string_lossy().into_owned();
    if s.len() + 1 > len.max(0) as usize {
        return false;
    }
    copy_out(&s, buf, len as usize);
    true
}
extern "C" fn u_level(_t: This) -> i32 {
    with_session(|s| s.level as i32).unwrap_or(0)
}
/// EUserHasLicenseForAppResult: 0 has it, 1 doesn't, 2 can't tell.
extern "C" fn u_has_license(_t: This, user: u64, app: u32) -> i32 {
    with_session(|s| if user != s.steam_id { 2 } else if s.app_id == app || s.owned.contains(&app) { 0 } else { 1 }).unwrap_or(2)
}
/// GetAuthSessionTicket(buffer, size, *written[, identity]): a ticket the
/// game hands its server (or EOS), which checks it with Steam. Steam must
/// accept the ticket list first; GetAuthSessionTicketResponse_t says when.
extern "C" fn u_auth_ticket(_t: This, buf: *mut u8, max: i32, len: *mut u32) -> u32 {
    put(len, 0);
    let Some(Reply::Ticket { ticket, crc, seq }) = crate::ipc::request(Request::AuthTicket { identity: String::new() }) else {
        crate::log!("no auth ticket");
        return 0;
    };
    if buf.is_null() || ticket.len() > max.max(0) as usize {
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(ticket.as_ptr(), buf, ticket.len()) };
    put(len, ticket.len() as u32);
    let handle = new_ticket_handle(crc);
    std::thread::spawn(move || {
        let ok = matches!(crate::ipc::request(Request::AuthTicketWait { seq }), Some(Reply::Ok));
        let mut b = Bytes::new(8);
        b.u32(0, handle).i32(4, if ok { state::RESULT_OK } else { state::RESULT_FAIL });
        lock().post(cb::GET_AUTH_SESSION_TICKET_RESPONSE, b.0);
    });
    handle
}

fn new_ticket_handle(crc: u32) -> u32 {
    let mut g = lock();
    let h = g.next_handle as u32;
    g.next_handle += 1;
    g.tickets.insert(h, crc);
    h
}

/// GetAuthTicketForWebApi(identity): the ticket arrives in
/// GetTicketForWebApiResponse_t.
extern "C" fn u_web_api_ticket(_t: This, identity: *const c_char) -> u32 {
    let identity = unsafe { text(identity) };
    let handle = {
        let mut g = lock();
        let h = g.next_handle as u32;
        g.next_handle += 1;
        h
    };
    std::thread::spawn(move || {
        let mut b = Bytes::new(2572);
        b.u32(0, handle);
        match crate::ipc::request(Request::AuthTicket { identity }) {
            Some(Reply::Ticket { ticket, crc, seq }) if ticket.len() <= 2560 => {
                lock().tickets.insert(handle, crc);
                let ok = matches!(crate::ipc::request(Request::AuthTicketWait { seq }), Some(Reply::Ok));
                b.i32(4, if ok { state::RESULT_OK } else { state::RESULT_FAIL }).i32(8, ticket.len() as i32);
                b.0[12..12 + ticket.len()].copy_from_slice(&ticket);
            }
            _ => {
                b.i32(4, state::RESULT_NO_CONNECTION);
            }
        }
        lock().post(cb::GET_TICKET_FOR_WEB_API_RESPONSE, b.0);
    });
    handle
}
extern "C" fn u_cancel_ticket(_t: This, handle: u32) {
    if let Some(crc) = lock().tickets.remove(&handle) {
        std::thread::spawn(move || crate::ipc::request(Request::CancelTicket { crc }));
    }
}
/// BeginAuthSession(ticket, size, player): we're the host checking a
/// player's ticket; the verdict comes in ValidateAuthTicketResponse_t.
extern "C" fn u_begin_auth(_t: This, ticket: *const u8, len: i32, user: u64) -> i32 {
    if ticket.is_null() || len <= 0 || len > 8192 {
        return 1; // k_EBeginAuthSessionResultInvalidTicket
    }
    let ticket = slice(ticket, len as usize).to_vec();
    std::thread::spawn(move || {
        let (response, owner) = match crate::ipc::request(Request::BeginAuth { steam_id: user, ticket }) {
            Some(Reply::AuthResult { response, owner }) => (response, owner),
            _ => (1, 0), // k_EAuthSessionResponseUserNotConnectedToSteam
        };
        crate::log!("ticket of {user}: response {response}");
        let mut b = Bytes::new(24);
        b.u64(0, user).i32(8, response as i32).u64(16, if owner == 0 { user } else { owner });
        lock().post(cb::VALIDATE_AUTH_TICKET_RESPONSE, b.0);
    });
    0 // k_EBeginAuthSessionResultOK
}
extern "C" fn u_end_auth(_t: This, user: u64) {
    std::thread::spawn(move || crate::ipc::request(Request::EndAuth { steam_id: user }));
}
extern "C" fn u_request_encrypted_ticket(_t: This, data: *const c_void, len: i32) -> u64 {
    let data = slice(data.cast(), len.max(0) as usize).to_vec();
    let call = lock().new_call();
    std::thread::spawn(move || {
        let result = match crate::ipc::request(Request::EncryptedTicket { data }) {
            Some(Reply::Bytes { data }) => {
                lock().encrypted_ticket = Some(data);
                state::RESULT_OK
            }
            _ => state::RESULT_NO_CONNECTION,
        };
        let mut b = Bytes::new(4);
        b.i32(0, result);
        lock().complete(call, cb::ENCRYPTED_APP_TICKET_RESPONSE, b.0, false);
    });
    call
}
extern "C" fn u_encrypted_ticket(_t: This, buf: *mut u8, max: i32, len: *mut u32) -> bool {
    let Some(t) = lock().encrypted_ticket.clone() else {
        put(len, 0);
        return false;
    };
    put(len, t.len() as u32);
    if buf.is_null() || t.len() > max.max(0) as usize {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(t.as_ptr(), buf, t.len()) };
    true
}

/// `<Steam>/userdata/<account id>/<app id>`.
fn user_dir() -> Option<PathBuf> {
    with_session(|s| {
        let root = PathBuf::from(&s.steam_root);
        root.join("userdata").join((s.steam_id & 0xFFFF_FFFF).to_string()).join(s.app_id.to_string())
    })
    .filter(|_| lock().session.as_ref().is_some_and(|s| !s.steam_root.is_empty()))
}

// ------------------------------------------------------------ ISteamUtils

extern "C" fn ut_seconds(_t: This) -> u32 {
    lock().started.elapsed().as_secs() as u32
}
extern "C" fn ut_real_time(_t: This) -> u32 {
    state::now()
}
extern "C" fn ut_country(_t: This) -> *const c_char {
    let mut g = lock();
    let c = g.session.as_ref().map(|s| s.country.clone()).unwrap_or_default();
    g.intern(&c)
}
extern "C" fn ut_battery(_t: This) -> u8 {
    255
}
extern "C" fn ut_app_id(_t: This) -> u32 {
    lock().app_id()
}
extern "C" fn ut_call_completed(_t: This, call: u64, failed: *mut bool) -> bool {
    let g = lock();
    match g.calls.get(&call) {
        Some(c) => {
            put(failed, c.failed);
            true
        }
        None => false,
    }
}
/// ESteamAPICallFailure: -1 none, 2 invalid handle.
extern "C" fn ut_call_failure(_t: This, call: u64) -> i32 {
    if lock().calls.contains_key(&call) { -1 } else { 2 }
}
extern "C" fn ut_call_result(_t: This, call: u64, buf: *mut u8, size: i32, expected: i32, failed: *mut bool) -> bool {
    call_result(call, buf, size, expected, failed)
}
extern "C" fn ut_ui_language(_t: This) -> *const c_char {
    let mut g = lock();
    let l = g.session.as_ref().map(|s| s.ui_language.clone()).unwrap_or_else(|| "english".into());
    g.intern(&l)
}
/// Newer FilterText(context, source, input, output, output size): no
/// filtering, the text goes through unchanged.
extern "C" fn ut_filter_text(_t: This, _ctx: i32, _source: u64, input: *const c_char, out: *mut c_char, len: u32) -> i32 {
    let s = unsafe { text(input) };
    copy_out(&s, out, len as usize);
    0
}

/// Hand a finished call's result to steam_api (used by Steam_GetAPICallResult too).
pub fn call_result(call: u64, buf: *mut u8, size: i32, expected: i32, failed: *mut bool) -> bool {
    let mut g = lock();
    let Some(c) = g.calls.get(&call) else { return false };
    if c.callback != expected {
        put(failed, true);
        return false;
    }
    let n = c.data.len().min(size.max(0) as usize);
    if !buf.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(c.data.as_ptr(), buf, n) };
    }
    put(failed, c.failed);
    g.calls.remove(&call);
    true
}

// ------------------------------------------------------------- ISteamApps

extern "C" fn a_game_language(_t: This) -> *const c_char {
    let mut g = lock();
    let l = g.session.as_ref().map(|s| s.game_language.clone()).unwrap_or_else(|| "english".into());
    g.intern(&l)
}
extern "C" fn a_owns(_t: This, app: u32) -> bool {
    with_session(|s| s.app_id == app || s.owned.contains(&app)).unwrap_or(false)
}
extern "C" fn a_has_dlc(_t: This, app: u32) -> bool {
    with_session(|s| s.dlc.iter().any(|d| d.0 == app)).unwrap_or(false)
}
extern "C" fn a_dlc_count(_t: This) -> i32 {
    with_session(|s| s.dlc.len() as i32).unwrap_or(0)
}
extern "C" fn a_dlc_data(_t: This, i: i32, app: *mut u32, available: *mut bool, name: *mut c_char, len: i32) -> bool {
    let Some(Some((id, n))) = with_session(|s| s.dlc.get(i.max(0) as usize).cloned()) else { return false };
    put(app, id);
    put(available, true);
    copy_out(&n, name, len.max(0) as usize);
    true
}
extern "C" fn a_beta_name(_t: This, buf: *mut c_char, len: i32) -> bool {
    match with_session(|s| s.beta.clone()).filter(|b| !b.is_empty()) {
        Some(b) => {
            copy_out(&b, buf, len.max(0) as usize);
            true
        }
        None => false,
    }
}
extern "C" fn a_install_dir(_t: This, app: u32, buf: *mut c_char, len: u32) -> u32 {
    match with_session(|s| (s.app_id == app).then(|| s.install_dir.clone())).flatten() {
        Some(dir) => copy_out(&dir, buf, len as usize) as u32,
        None => 0,
    }
}
extern "C" fn a_installed(_t: This, app: u32) -> bool {
    lock().app_id() == app
}
extern "C" fn a_build_id(_t: This) -> i32 {
    with_session(|s| s.build_id as i32).unwrap_or(0)
}

// ---------------------------------------------------------- ISteamFriends

extern "C" fn fr_persona_name(_t: This) -> *const c_char {
    let mut g = lock();
    let n = g.session.as_ref().map(|s| s.persona_name.clone()).unwrap_or_default();
    g.intern(&n)
}

/// EFriendFlags: 0x04 friends, 0x08 requests to you, 0x80 requests you sent.
fn flag_match(relationship: u32, flags: i32) -> bool {
    let want = match relationship {
        3 => 0x04,
        2 => 0x08,
        4 => 0x80,
        1 => 0x20,
        5 => 0x40,
        _ => 0,
    };
    want & flags != 0
}

fn friends_matching(g: &Global, flags: i32) -> Vec<u64> {
    g.session.as_ref().map(|s| s.friends.iter().filter(|f| flag_match(f.relationship, flags)).map(|f| f.steam_id).collect()).unwrap_or_default()
}

extern "C" fn fr_count(_t: This, flags: i32) -> i32 {
    friends_matching(&lock(), flags).len() as i32
}
extern "C" fn fr_by_index(_t: This, ret: *mut u64, i: i32, flags: i32) -> *mut u64 {
    let id = friends_matching(&lock(), flags).get(i.max(0) as usize).copied().unwrap_or(0);
    put(ret, id);
    ret
}
extern "C" fn fr_relationship(_t: This, id: u64) -> i32 {
    with_session(|s| s.friends.iter().find(|f| f.steam_id == id).map(|f| f.relationship as i32).unwrap_or(0)).unwrap_or(0)
}
extern "C" fn fr_state(_t: This, id: u64) -> i32 {
    with_session(|s| if id == s.steam_id { 1 } else { s.friends.iter().find(|f| f.steam_id == id).map(|f| f.state as i32).unwrap_or(0) }).unwrap_or(0)
}
extern "C" fn fr_name(_t: This, id: u64) -> *const c_char {
    let mut g = lock();
    let name = g
        .session
        .as_ref()
        .map(|s| if id == s.steam_id { s.persona_name.clone() } else { s.friends.iter().find(|f| f.steam_id == id).map(|f| f.name.clone()).unwrap_or_default() })
        .unwrap_or_default();
    g.intern(&name)
}
extern "C" fn fr_game_played(_t: This, id: u64, out: *mut u8) -> bool {
    let game = with_session(|s| if id == s.steam_id { s.app_id } else { s.friends.iter().find(|f| f.steam_id == id).map(|f| f.game).unwrap_or(0) }).unwrap_or(0);
    if game == 0 {
        return false;
    }
    if !out.is_null() {
        // FriendGameInfo_t: CGameID, IP, game port, query port, lobby.
        let mut b = Bytes::new(24);
        b.u64(0, game as u64);
        unsafe { std::ptr::copy_nonoverlapping(b.0.as_ptr(), out, 24) };
    }
    true
}
extern "C" fn fr_level(_t: This, id: u64) -> i32 {
    with_session(|s| if id == s.steam_id { s.level as i32 } else { 0 }).unwrap_or(0)
}
extern "C" fn fr_has_friend(_t: This, id: u64, flags: i32) -> bool {
    with_session(|s| s.friends.iter().any(|f| f.steam_id == id && flag_match(f.relationship, flags))).unwrap_or(false)
}
/// No overlay: open the page in the browser instead.
extern "C" fn fr_overlay_web(_t: This, url: *const c_char, _mode: i32) {
    crate::win::open_url(&unsafe { text(url) });
}
extern "C" fn fr_overlay_store(_t: This, app: u32, _flag: i32) {
    crate::win::open_url(&format!("https://store.steampowered.com/app/{app}/"));
}
extern "C" fn fr_set_rich_presence(_t: This, key: *const c_char, value: *const c_char) -> bool {
    let (k, v) = unsafe { (text(key), text(value)) };
    if k.is_empty() || k.len() > 64 || v.len() > 256 {
        return false;
    }
    {
        let mut g = lock();
        g.rich.retain(|(kk, _)| *kk != k);
        if !v.is_empty() {
            if g.rich.len() >= 30 {
                return false;
            }
            g.rich.push((k, v));
        }
    }
    send_rich_presence();
    true
}
extern "C" fn fr_clear_rich_presence(_t: This) {
    lock().rich.clear();
    send_rich_presence();
}
extern "C" fn fr_rich_presence(_t: This, id: u64, key: *const c_char) -> *const c_char {
    let k = unsafe { text(key) };
    let mut g = lock();
    let v = if id == g.me() { g.rich.iter().find(|(kk, _)| kk.eq_ignore_ascii_case(&k)).map(|(_, v)| v.clone()).unwrap_or_default() } else { String::new() };
    g.intern(&v)
}
extern "C" fn fr_rich_presence_count(_t: This, id: u64) -> i32 {
    let g = lock();
    if id == g.me() { g.rich.len() as i32 } else { 0 }
}
extern "C" fn fr_rich_presence_key(_t: This, id: u64, i: i32) -> *const c_char {
    let mut g = lock();
    let k = if id == g.me() { g.rich.get(i.max(0) as usize).map(|(k, _)| k.clone()).unwrap_or_default() } else { String::new() };
    g.intern(&k)
}

/// Rich presence goes up in the background, newest state last.
fn send_rich_presence() {
    static SENDING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    std::thread::spawn(|| {
        let _one_at_a_time = SENDING.lock();
        let pairs = lock().rich.clone();
        let _ = crate::ipc::request(Request::RichPresence { pairs });
    });
}

// -------------------------------------------------------- ISteamUserStats

extern "C" fn st_request_current(_t: This) -> bool {
    let mut g = lock();
    if g.session.is_none() {
        return false;
    }
    let (app, me, ok) = (g.app_id(), g.me(), g.stats.loaded);
    g.post(cb::USER_STATS_RECEIVED, state::user_stats_received(app, if ok { state::RESULT_OK } else { state::RESULT_FAIL }, me));
    true
}
extern "C" fn st_get_int(_t: This, name: *const c_char, out: *mut i32) -> bool {
    let n = unsafe { text(name) };
    match lock().stats.stat(&n) {
        Some(s) if s.def.kind == 1 => {
            put(out, s.value as i32);
            true
        }
        _ => false,
    }
}
extern "C" fn st_get_float(_t: This, name: *const c_char, out: *mut f32) -> bool {
    let n = unsafe { text(name) };
    match lock().stats.stat(&n) {
        Some(s) if s.def.kind == 2 || s.def.kind == 3 => {
            put(out, f32::from_bits(s.value));
            true
        }
        _ => false,
    }
}
fn in_range(v: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m)
}
extern "C" fn st_set_int(_t: This, name: *const c_char, v: i32) -> bool {
    let n = unsafe { text(name) };
    let mut g = lock();
    let Some(s) = g.stats.stat(&n) else { return false };
    if s.def.kind != 1 || !in_range(v as f64, s.def.min, s.def.max) || (s.def.increment_only && v < s.value as i32) {
        return false;
    }
    s.value = v as u32;
    true
}
extern "C" fn st_set_float(_t: This, name: *const c_char, v: f32) -> bool {
    let n = unsafe { text(name) };
    let mut g = lock();
    let Some(s) = g.stats.stat(&n) else { return false };
    if s.def.kind != 2 || !v.is_finite() || !in_range(v as f64, s.def.min, s.def.max) || (s.def.increment_only && v < f32::from_bits(s.value)) {
        return false;
    }
    s.value = v.to_bits();
    true
}
extern "C" fn st_update_avg(_t: This, name: *const c_char, count: f32, seconds: f64) -> bool {
    let n = unsafe { text(name) };
    let mut g = lock();
    let Some(s) = g.stats.stat(&n) else { return false };
    if s.def.kind != 3 || seconds <= 0.0 {
        return false;
    }
    s.value = ((count as f64 / seconds) as f32).to_bits();
    true
}
extern "C" fn st_get_ach(_t: This, name: *const c_char, out: *mut bool) -> bool {
    let n = unsafe { text(name) };
    let g = lock();
    match g.stats.ach(&n) {
        Some(i) => {
            put(out, g.stats.achs[i].unlocked);
            true
        }
        None => false,
    }
}
extern "C" fn st_get_ach_time(_t: This, name: *const c_char, out: *mut bool, time: *mut u32) -> bool {
    let n = unsafe { text(name) };
    let g = lock();
    match g.stats.ach(&n) {
        Some(i) => {
            put(out, g.stats.achs[i].unlocked);
            put(time, g.stats.achs[i].time);
            true
        }
        None => false,
    }
}
extern "C" fn st_set_ach(_t: This, name: *const c_char) -> bool {
    set_ach(name, true)
}
extern "C" fn st_clear_ach(_t: This, name: *const c_char) -> bool {
    set_ach(name, false)
}
fn set_ach(name: *const c_char, on: bool) -> bool {
    let n = unsafe { text(name) };
    let mut g = lock();
    match g.stats.ach(&n) {
        Some(i) => {
            g.stats.set_ach(i, on);
            crate::log!("achievement {n} -> {on}");
            true
        }
        None => false,
    }
}
extern "C" fn st_ach_attr(_t: This, name: *const c_char, key: *const c_char) -> *const c_char {
    let (n, k) = unsafe { (text(name), text(key)) };
    let mut g = lock();
    let v = g
        .stats
        .ach(&n)
        .map(|i| {
            let a = &g.stats.achs[i].def;
            match k.as_str() {
                "name" => a.display_name.clone(),
                "desc" => a.description.clone(),
                "hidden" => if a.hidden { "1" } else { "0" }.to_string(),
                _ => String::new(),
            }
        })
        .unwrap_or_default();
    g.intern(&v)
}
extern "C" fn st_indicate_progress(_t: This, name: *const c_char, cur: u32, max: u32) -> bool {
    let n = unsafe { text(name) };
    let mut g = lock();
    let Some(i) = g.stats.ach(&n) else { return false };
    if g.stats.achs[i].unlocked {
        return false;
    }
    let app = g.app_id();
    let data = achievement_stored(app, &g.stats.achs[i].def.name, cur, max);
    g.post(cb::USER_ACHIEVEMENT_STORED, data);
    true
}
extern "C" fn st_num_achs(_t: This) -> u32 {
    lock().stats.achs.len() as u32
}
extern "C" fn st_ach_name(_t: This, i: u32) -> *const c_char {
    let mut g = lock();
    let n = g.stats.achs.get(i as usize).map(|a| a.def.name.clone());
    match n {
        Some(n) => g.intern(&n),
        None => std::ptr::null(),
    }
}

/// UserAchievementStored_t: game id, group flag, name[128], progress.
fn achievement_stored(app: u32, name: &str, cur: u32, max: u32) -> Vec<u8> {
    let mut b = Bytes::new(152);
    b.u64(0, app as u64).str(9, 128, name).u32(140, cur).u32(144, max);
    b.0
}

/// UserStatsStored_t: game id, result.
fn stats_stored(app: u32, result: i32) -> Vec<u8> {
    let mut b = Bytes::new(16);
    b.u64(0, app as u64).i32(8, result);
    b.0
}

extern "C" fn st_store(_t: This) -> bool {
    store_stats()
}

fn store_stats() -> bool {
    let (changes, crc, reset) = {
        let mut g = lock();
        if !g.stats.loaded {
            return false;
        }
        if g.stats.storing {
            g.stats.store_again = true;
            return true;
        }
        let changes: Vec<(u32, u32)> = g.stats.stats.iter().filter(|s| s.value != s.stored).map(|s| (s.def.id, s.value)).collect();
        let reset = std::mem::take(&mut g.stats.reset);
        if changes.is_empty() && !reset {
            let app = g.app_id();
            g.post(cb::USER_STATS_STORED, stats_stored(app, state::RESULT_OK));
            return true;
        }
        g.stats.storing = true;
        (changes, g.stats.crc, reset)
    };
    crate::log!("storing {} stat(s)", changes.len());
    std::thread::spawn(move || {
        let reply = crate::ipc::request(Request::StoreStats { stats: changes.clone(), crc, reset });
        let again = {
            let mut g = lock();
            let app = g.app_id();
            g.stats.storing = false;
            match reply {
                Some(Reply::Stored { eresult, crc }) if eresult == state::RESULT_OK => {
                    g.stats.crc = crc;
                    for (id, v) in &changes {
                        if let Some(s) = g.stats.stats.iter_mut().find(|s| s.def.id == *id) {
                            s.stored = *v;
                        }
                    }
                    let mut unlocked = Vec::new();
                    for a in g.stats.achs.iter_mut() {
                        let Some((_, v)) = changes.iter().find(|(id, _)| *id == a.def.block) else { continue };
                        let on = (v >> a.def.bit) & 1 == 1;
                        if on && !a.stored {
                            unlocked.push(a.def.name.clone());
                        }
                        a.stored = on;
                    }
                    for name in unlocked {
                        crate::log!("achievement {name} stored");
                        g.post(cb::USER_ACHIEVEMENT_STORED, achievement_stored(app, &name, 0, 0));
                    }
                    g.post(cb::USER_STATS_STORED, stats_stored(app, state::RESULT_OK));
                }
                other => {
                    crate::log!("storing stats failed: {other:?}");
                    let code = match other {
                        Some(Reply::Stored { eresult, .. }) => eresult,
                        _ => state::RESULT_NO_CONNECTION,
                    };
                    g.post(cb::USER_STATS_STORED, stats_stored(app, code));
                }
            }
            std::mem::take(&mut g.stats.store_again)
        };
        if again {
            store_stats();
        }
    });
    true
}

extern "C" fn st_request_user(_t: This, user: u64) -> u64 {
    let mut g = lock();
    let (app, me) = (g.app_id(), g.me());
    let call = g.new_call();
    if user == me && g.stats.loaded {
        g.complete(call, cb::USER_STATS_RECEIVED, state::user_stats_received(app, state::RESULT_OK, user), false);
        return call;
    }
    drop(g);
    std::thread::spawn(move || {
        let reply = crate::ipc::request(Request::UserStats { steam_id: user });
        let mut g = lock();
        let result = match reply {
            Some(Reply::Stats { result: Ok(stats) }) => {
                g.other_stats.insert(user, stats);
                state::RESULT_OK
            }
            _ => state::RESULT_FAIL,
        };
        let data = state::user_stats_received(app, result, user);
        g.complete(call, cb::USER_STATS_RECEIVED, data.clone(), false);
        g.post(cb::USER_STATS_RECEIVED, data);
    });
    call
}

/// Look at another user's (or our own) loaded stats.
fn user_stat<R>(user: u64, name: &str, f: impl FnOnce(u8, u32) -> R) -> Option<R> {
    let mut g = lock();
    if user == g.me() {
        let s = g.stats.stat(name)?;
        return Some(f(s.def.kind, s.value));
    }
    let s = g.other_stats.get(&user)?.stats.iter().find(|s| s.kind != 4 && s.name.eq_ignore_ascii_case(name))?;
    Some(f(s.kind, s.value))
}
fn user_ach(user: u64, name: &str) -> Option<(bool, u32)> {
    let g = lock();
    if user == g.me() {
        let i = g.stats.ach(name)?;
        return Some((g.stats.achs[i].unlocked, g.stats.achs[i].time));
    }
    let a = g.other_stats.get(&user)?.achievements.iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
    Some((a.unlocked, a.unlock_time))
}
extern "C" fn st_user_int(_t: This, user: u64, name: *const c_char, out: *mut i32) -> bool {
    let n = unsafe { text(name) };
    match user_stat(user, &n, |kind, v| (kind == 1).then_some(v as i32)).flatten() {
        Some(v) => {
            put(out, v);
            true
        }
        None => false,
    }
}
extern "C" fn st_user_float(_t: This, user: u64, name: *const c_char, out: *mut f32) -> bool {
    let n = unsafe { text(name) };
    match user_stat(user, &n, |kind, v| (kind == 2 || kind == 3).then_some(f32::from_bits(v))).flatten() {
        Some(v) => {
            put(out, v);
            true
        }
        None => false,
    }
}
extern "C" fn st_user_ach(_t: This, user: u64, name: *const c_char, out: *mut bool) -> bool {
    let n = unsafe { text(name) };
    match user_ach(user, &n) {
        Some((on, _)) => {
            put(out, on);
            true
        }
        None => false,
    }
}
extern "C" fn st_user_ach_time(_t: This, user: u64, name: *const c_char, out: *mut bool, time: *mut u32) -> bool {
    let n = unsafe { text(name) };
    match user_ach(user, &n) {
        Some((on, t)) => {
            put(out, on);
            put(time, t);
            true
        }
        None => false,
    }
}
extern "C" fn st_reset(_t: This, achievements_too: bool) -> bool {
    let mut g = lock();
    if !g.stats.loaded {
        return false;
    }
    for s in g.stats.stats.iter_mut().filter(|s| s.def.kind != 4) {
        s.value = s.def.default;
    }
    if achievements_too {
        for i in 0..g.stats.achs.len() {
            g.stats.set_ach(i, false);
        }
    }
    g.stats.reset = true;
    true
}

extern "C" fn st_attach_ugc(_t: This, lb: u64, _ugc: u64) -> u64 {
    let mut b = Bytes::new(16);
    b.u64(0, lb).i32(8, state::RESULT_FAIL);
    lock().call_now(cb::LEADERBOARD_UGC_SET, b.0)
}
extern "C" fn st_current_players(_t: This) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let count = match crate::ipc::request(Request::CurrentPlayers) {
            Some(Reply::Players { count }) => count,
            _ => None,
        };
        let mut b = Bytes::new(8);
        b.u8(0, count.is_some() as u8).i32(4, count.unwrap_or(0));
        lock().complete(call, cb::NUMBER_OF_CURRENT_PLAYERS, b.0, false);
    });
    call
}
extern "C" fn st_global_stats(_t: This, _days: i32) -> u64 {
    let mut g = lock();
    let mut b = Bytes::new(16);
    b.u64(0, g.app_id() as u64).i32(8, state::RESULT_FAIL);
    g.call_now(cb::GLOBAL_STATS_RECEIVED, b.0)
}

// ---------------------------------------------------- ISteamRemoteStorage

/// `<Steam>/userdata/<account>/<app>/remote`: where Steam keeps a game's
/// cloud files; SteamInRust syncs this folder before and after playing.
fn remote_dir() -> Option<PathBuf> {
    user_dir().map(|d| d.join("remote"))
}

/// A cloud file name as a path inside the remote folder (no escaping it).
fn remote_path(name: *const c_char) -> Option<PathBuf> {
    let n = unsafe { text(name) }.replace('\\', "/");
    if n.is_empty() || n.starts_with('/') || n.contains(':') || n.split('/').any(|p| p == ".." || p.is_empty()) {
        return None;
    }
    Some(remote_dir()?.join(n))
}

fn write_file(path: &std::path::Path, data: &[u8]) -> bool {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write next to it, then swap in, so a crash never leaves half a save.
    let tmp = path.with_extension("steaminrust-tmp");
    std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, path)).is_ok()
}

pub(crate) fn slice<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, len) } }
}

extern "C" fn rs_write(_t: This, name: *const c_char, data: *const u8, len: i32) -> bool {
    let Some(path) = remote_path(name) else { return false };
    if len < 0 {
        return false;
    }
    write_file(&path, slice(data, len as usize))
}
extern "C" fn rs_read(_t: This, name: *const c_char, buf: *mut u8, len: i32) -> i32 {
    let Some(data) = remote_path(name).and_then(|p| std::fs::read(p).ok()) else { return 0 };
    let n = data.len().min(len.max(0) as usize);
    if !buf.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf, n) };
    }
    n as i32
}
extern "C" fn rs_write_async(_t: This, name: *const c_char, data: *const u8, len: u32) -> u64 {
    let ok = remote_path(name).is_some_and(|p| write_file(&p, slice(data, len as usize)));
    let mut b = Bytes::new(4);
    b.i32(0, if ok { state::RESULT_OK } else { state::RESULT_FAIL });
    lock().call_now(cb::REMOTE_STORAGE_FILE_WRITE_ASYNC_COMPLETE, b.0)
}
extern "C" fn rs_read_async(_t: This, name: *const c_char, offset: u32, len: u32) -> u64 {
    let data = remote_path(name).and_then(|p| std::fs::read(p).ok());
    let mut g = lock();
    let call = g.new_call();
    let mut b = Bytes::new(24);
    b.u64(0, call);
    match data {
        Some(d) if (offset as usize) <= d.len() => {
            let end = (offset as usize + len as usize).min(d.len());
            let part = d[offset as usize..end].to_vec();
            b.i32(8, state::RESULT_OK).u32(12, offset).u32(16, part.len() as u32);
            g.read_results.insert(call, part);
        }
        _ => {
            b.i32(8, state::RESULT_FILE_NOT_FOUND);
        }
    }
    g.complete(call, cb::REMOTE_STORAGE_FILE_READ_ASYNC_COMPLETE, b.0, false);
    call
}
extern "C" fn rs_read_async_complete(_t: This, call: u64, buf: *mut u8, len: u32) -> bool {
    let Some(data) = lock().read_results.remove(&call) else { return false };
    if buf.is_null() || (len as usize) < data.len() {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len()) };
    true
}
extern "C" fn rs_delete(_t: This, name: *const c_char) -> bool {
    remote_path(name).is_some_and(|p| std::fs::remove_file(p).is_ok())
}
extern "C" fn rs_share(_t: This, name: *const c_char) -> u64 {
    let n = unsafe { text(name) };
    let mut b = Bytes::new(280);
    b.i32(0, state::RESULT_FAIL).str(16, 260, &n);
    lock().call_now(cb::REMOTE_STORAGE_FILE_SHARE_RESULT, b.0)
}
extern "C" fn rs_stream_open(_t: This, name: *const c_char) -> u64 {
    let n = unsafe { text(name) };
    if remote_path(name).is_none() {
        return u64::MAX;
    }
    let mut g = lock();
    let h = g.next_handle;
    g.next_handle += 1;
    g.streams.insert(h, (n, Vec::new()));
    h
}
extern "C" fn rs_stream_write(_t: This, h: u64, data: *const u8, len: i32) -> bool {
    let mut g = lock();
    match g.streams.get_mut(&h) {
        Some((_, buf)) if len >= 0 => {
            buf.extend_from_slice(slice(data, len as usize));
            true
        }
        _ => false,
    }
}
extern "C" fn rs_stream_close(_t: This, h: u64) -> bool {
    let Some((name, data)) = lock().streams.remove(&h) else { return false };
    let Ok(c) = std::ffi::CString::new(name) else { return false };
    remote_path(c.as_ptr()).is_some_and(|p| write_file(&p, &data))
}
extern "C" fn rs_stream_cancel(_t: This, h: u64) -> bool {
    lock().streams.remove(&h).is_some()
}
extern "C" fn rs_exists(_t: This, name: *const c_char) -> bool {
    remote_path(name).is_some_and(|p| p.is_file())
}
extern "C" fn rs_size(_t: This, name: *const c_char) -> i32 {
    remote_path(name).and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len().min(i32::MAX as u64) as i32).unwrap_or(0)
}
extern "C" fn rs_timestamp(_t: This, name: *const c_char) -> i64 {
    remote_path(name)
        .and_then(|p| std::fs::metadata(p).ok()?.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn list_remote() -> Vec<(String, i32)> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(String, i32)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if let (Ok(rel), Ok(m)) = (p.strip_prefix(base), e.metadata()) {
                if p.extension().is_some_and(|x| x == "steaminrust-tmp") {
                    continue;
                }
                out.push((rel.to_string_lossy().replace('\\', "/"), m.len().min(i32::MAX as u64) as i32));
            }
        }
    }
    let mut out = Vec::new();
    if let Some(dir) = remote_dir() {
        walk(&dir, &dir, &mut out);
    }
    out.sort();
    out
}

extern "C" fn rs_count(_t: This) -> i32 {
    let files = list_remote();
    let mut g = lock();
    g.files = files.into_iter().filter_map(|(n, s)| Some((std::ffi::CString::new(n).ok()?, s))).collect();
    g.files.len() as i32
}
extern "C" fn rs_name_and_size(_t: This, i: i32, size: *mut i32) -> *const c_char {
    if lock().files.is_empty() {
        rs_count(_t);
    }
    let mut g = lock();
    let Some((name, s)) = g.files.get(i.max(0) as usize).map(|(n, s)| (n.to_string_lossy().into_owned(), *s)) else {
        put(size, 0);
        return c"".as_ptr();
    };
    put(size, s);
    g.intern(&name)
}
extern "C" fn rs_quota(_t: This, total: *mut u64, available: *mut u64) -> bool {
    let used: u64 = list_remote().iter().map(|(_, s)| *s as u64).sum();
    let quota = 1u64 << 30;
    put(total, quota);
    put(available, quota.saturating_sub(used));
    true
}

// -------------------------------------------------- ISteamNetworkingUtils

extern "C" fn nu_timestamp(_t: This) -> i64 {
    lock().started.elapsed().as_micros() as i64 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friend_flags() {
        assert!(flag_match(3, 0x04));
        assert!(!flag_match(3, 0x08));
        assert!(flag_match(2, 0xFFFF));
    }

    #[test]
    fn copy_out_truncates() {
        let mut buf = [1 as c_char; 4];
        assert_eq!(copy_out("hello", buf.as_mut_ptr(), 4), 4);
        assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap(), "hel");
    }

    #[test]
    fn callback_layouts() {
        let a = achievement_stored(480, "ACH_WIN", 0, 0);
        assert_eq!(a.len(), 152);
        assert_eq!(&a[9..16], b"ACH_WIN");
        assert_eq!(stats_stored(480, 1).len(), 16);
    }
}

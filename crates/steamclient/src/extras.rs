//! Leaderboards, global achievement percentages, avatars and ISteamHTTP.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, copy_out, put, slice, text};
use crate::protocol::{LbEntry, Reply, Request};
use crate::state::{self, Bytes, lock};
use crate::vt::Fp;
use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{LazyLock, Mutex, MutexGuard};

pub fn imp(kind: &str, name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match (kind, name) {
        ("ISteamUserStats", "FindOrCreateLeaderboard") => f!(lb_find_or_create),
        ("ISteamUserStats", "FindLeaderboard") => f!(lb_find),
        ("ISteamUserStats", "GetLeaderboardName") => f!(lb_name),
        ("ISteamUserStats", "GetLeaderboardEntryCount") => f!(lb_count),
        ("ISteamUserStats", "GetLeaderboardSortMethod") => f!(lb_sort),
        ("ISteamUserStats", "GetLeaderboardDisplayType") => f!(lb_display),
        ("ISteamUserStats", "DownloadLeaderboardEntries") => f!(lb_download),
        ("ISteamUserStats", "DownloadLeaderboardEntriesForUsers") => f!(lb_download_users),
        ("ISteamUserStats", "GetDownloadedLeaderboardEntry") => f!(lb_entry),
        ("ISteamUserStats", "UploadLeaderboardScore") => f!(lb_upload),
        ("ISteamUserStats", "RequestGlobalAchievementPercentages") => f!(request_percentages),
        ("ISteamUserStats", "GetAchievementAchievedPercent") => f!(achieved_percent),
        ("ISteamUserStats", "GetMostAchievedAchievementInfo") => f!(most_achieved),
        ("ISteamUserStats", "GetNextMostAchievedAchievementInfo") => f!(next_most_achieved),
        ("ISteamFriends", "GetSmallFriendAvatar") => f!(avatar_small),
        ("ISteamFriends", "GetMediumFriendAvatar") => f!(avatar_medium),
        ("ISteamFriends", "GetLargeFriendAvatar") => f!(avatar_large),
        ("ISteamUtils", "GetImageSize") => f!(image_size),
        ("ISteamUtils", "GetImageRGBA") => f!(image_rgba),
        ("ISteamHTTP", m) => http_imp(m),
        ("ISteamUGC", m) => crate::ugc::imp(m),
        ("ISteamInventory", m) => crate::inventory::imp(m),
        ("ISteamMatchmakingServers", m) => crate::servers::imp(m),
        ("ISteamScreenshots", m) => crate::screenshots::imp(m),
        ("ISteamInput" | "ISteamController", m) => crate::input::imp(m),
        _ => None,
    }
}

#[derive(Default)]
struct Extras {
    boards: HashMap<u64, (String, i32, i32, i32)>,
    entry_sets: HashMap<u64, Vec<LbEntry>>,
    next: u64,
    percentages: Vec<(String, f32)>,
    avatars: HashMap<(u64, u8), i32>,
    images: HashMap<i32, (u32, u32, Vec<u8>)>,
    http: HashMap<u32, Http>,
}

#[derive(Default)]
struct Http {
    method: i32,
    url: String,
    context: u64,
    timeout_ms: u32,
    headers: Vec<(String, String)>,
    params: Vec<(String, String)>,
    body: Option<(String, Vec<u8>)>,
    response: Option<(i32, Vec<(String, String)>, Vec<u8>, bool)>,
}

static X: LazyLock<Mutex<Extras>> = LazyLock::new(|| Mutex::new(Extras { next: 1, ..Default::default() }));

/// Lock order: these first, then the global state.
fn x() -> MutexGuard<'static, Extras> {
    X.lock().unwrap_or_else(|e| e.into_inner())
}

fn next_id(e: &mut Extras) -> u64 {
    let id = e.next;
    e.next += 1;
    id
}

// --------------------------------------------------------- leaderboards

const LEADERBOARD_FIND_RESULT: i32 = 1104;
const LEADERBOARD_SCORES_DOWNLOADED: i32 = 1105;
const LEADERBOARD_SCORE_UPLOADED: i32 = 1106;
const GLOBAL_ACHIEVEMENT_PERCENTAGES_READY: i32 = 1110;
const AVATAR_IMAGE_LOADED: i32 = 334;
const HTTP_REQUEST_COMPLETED: i32 = 2101;

fn find(name: String, create: bool, sort: i32, display: i32) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let found = match crate::ipc::request(Request::LbFind { name, create, sort, display }) {
            Some(Reply::LbFound { found: true, id, count, sort, display, name }) => {
                x().boards.insert(id as u64, (name, count, sort, display));
                Some(id as u64)
            }
            _ => None,
        };
        let mut b = Bytes::new(16);
        b.u64(0, found.unwrap_or(0)).u8(8, found.is_some() as u8);
        lock().complete(call, LEADERBOARD_FIND_RESULT, b.0, false);
    });
    call
}

extern "C" fn lb_find_or_create(_t: This, name: *const c_char, sort: i32, display: i32) -> u64 {
    find(unsafe { text(name) }, true, sort, display)
}
extern "C" fn lb_find(_t: This, name: *const c_char) -> u64 {
    find(unsafe { text(name) }, false, 0, 0)
}
extern "C" fn lb_name(_t: This, lb: u64) -> *const c_char {
    let n = x().boards.get(&lb).map(|b| b.0.clone()).unwrap_or_default();
    lock().intern(&n)
}
extern "C" fn lb_count(_t: This, lb: u64) -> i32 {
    x().boards.get(&lb).map(|b| b.1).unwrap_or(0)
}
extern "C" fn lb_sort(_t: This, lb: u64) -> i32 {
    x().boards.get(&lb).map(|b| b.2).unwrap_or(0)
}
extern "C" fn lb_display(_t: This, lb: u64) -> i32 {
    x().boards.get(&lb).map(|b| b.3).unwrap_or(0)
}

fn download(lb: u64, request: i32, start: i32, end: i32, users: Vec<u64>) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let entries = match crate::ipc::request(Request::LbEntries { lb: lb as i32, request, start, end, users }) {
            Some(Reply::LbEntries { total, entries }) => {
                if let Some(b) = x().boards.get_mut(&lb) {
                    b.1 = total;
                }
                entries
            }
            _ => Vec::new(),
        };
        let n = entries.len() as i32;
        let handle = {
            let mut e = x();
            let h = next_id(&mut e);
            e.entry_sets.insert(h, entries);
            h
        };
        let mut b = Bytes::new(24);
        b.u64(0, lb).u64(8, handle).i32(16, n);
        lock().complete(call, LEADERBOARD_SCORES_DOWNLOADED, b.0, false);
    });
    call
}

extern "C" fn lb_download(_t: This, lb: u64, request: i32, start: i32, end: i32) -> u64 {
    download(lb, request, start, end, Vec::new())
}
extern "C" fn lb_download_users(_t: This, lb: u64, users: *const u64, n: i32) -> u64 {
    let ids: Vec<u64> = slice(users.cast(), n.max(0) as usize * 8).chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
    download(lb, 3, 0, 0, ids)
}
/// LeaderboardEntry_t: user, global rank, score, detail count, UGC handle.
extern "C" fn lb_entry(_t: This, set: u64, index: i32, out: *mut u8, details: *mut i32, max: i32) -> bool {
    let Some(e) = x().entry_sets.get(&set).and_then(|s| s.get(index.max(0) as usize).cloned()) else { return false };
    if !out.is_null() {
        let mut b = Bytes::new(32);
        b.u64(0, e.steam_id).i32(8, e.rank).i32(12, e.score).i32(16, e.details.len() as i32).u64(24, e.ugc);
        unsafe { std::ptr::copy_nonoverlapping(b.0.as_ptr(), out, 32) };
    }
    if !details.is_null() {
        for (i, d) in e.details.iter().take(max.max(0) as usize).enumerate() {
            unsafe { details.add(i).write_unaligned(*d) };
        }
    }
    true
}
extern "C" fn lb_upload(_t: This, lb: u64, method: i32, score: i32, details: *const i32, n: i32) -> u64 {
    let details: Vec<i32> = slice(details.cast(), n.clamp(0, 64) as usize * 4).chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
    let call = lock().new_call();
    std::thread::spawn(move || {
        let (ok, changed, new, prev) = match crate::ipc::request(Request::LbUpload { lb: lb as i32, method, score, details }) {
            Some(Reply::LbUploaded { ok, changed, rank_new, rank_prev }) => (ok, changed, rank_new, rank_prev),
            _ => (false, false, 0, 0),
        };
        let mut b = Bytes::new(32);
        b.u8(0, ok as u8).u64(8, lb).i32(16, score).u8(20, changed as u8).i32(24, new).i32(28, prev);
        lock().complete(call, LEADERBOARD_SCORE_UPLOADED, b.0, false);
    });
    call
}

// ------------------------------------------- achievement percentages

extern "C" fn request_percentages(_t: This) -> u64 {
    let (call, app) = {
        let mut g = lock();
        (g.new_call(), g.app_id())
    };
    std::thread::spawn(move || {
        let mut list = match crate::ipc::request(Request::GlobalPercentages) {
            Some(Reply::Percentages { list }) => list,
            _ => Vec::new(),
        };
        list.sort_by(|a, b| b.1.total_cmp(&a.1));
        let ok = !list.is_empty();
        x().percentages = list;
        let mut b = Bytes::new(16);
        b.u64(0, app as u64).i32(8, if ok { state::RESULT_OK } else { state::RESULT_FAIL });
        lock().complete(call, GLOBAL_ACHIEVEMENT_PERCENTAGES_READY, b.0, false);
    });
    call
}

extern "C" fn achieved_percent(_t: This, name: *const c_char, out: *mut f32) -> bool {
    let n = unsafe { text(name) };
    match x().percentages.iter().find(|(k, _)| k.eq_ignore_ascii_case(&n)) {
        Some((_, p)) => {
            put(out, *p);
            true
        }
        None => false,
    }
}

fn nth_achieved(i: i32, name: *mut c_char, len: u32, pct: *mut f32, achieved: *mut bool) -> i32 {
    let Some((n, p)) = x().percentages.get(i.max(0) as usize).cloned() else { return -1 };
    copy_out(&n, name, len as usize);
    put(pct, p);
    let mine = {
        let g = lock();
        g.stats.ach(&n).map(|a| g.stats.achs[a].unlocked).unwrap_or(false)
    };
    put(achieved, mine);
    i
}
extern "C" fn most_achieved(_t: This, name: *mut c_char, len: u32, pct: *mut f32, achieved: *mut bool) -> i32 {
    nth_achieved(0, name, len, pct, achieved)
}
extern "C" fn next_most_achieved(_t: This, prev: i32, name: *mut c_char, len: u32, pct: *mut f32, achieved: *mut bool) -> i32 {
    if prev < 0 { -1 } else { nth_achieved(prev + 1, name, len, pct, achieved) }
}

// ---------------------------------------------------------------- avatars

/// Image handle for an avatar, loading it in the background the first time
/// (AvatarImageLoaded_t says when it's there).
fn avatar(user: u64, size: u8, loading: i32) -> i32 {
    if user == 0 {
        return 0;
    }
    {
        let mut e = x();
        match e.avatars.get(&(user, size)) {
            Some(&h) => return h,
            None => {
                e.avatars.insert((user, size), loading);
            }
        }
    }
    std::thread::spawn(move || {
        let handle = match crate::ipc::request(Request::Avatar { steam_id: user, size }) {
            Some(Reply::Image { width, height, rgba }) if rgba.len() == (width * height * 4) as usize => {
                let mut e = x();
                let h = next_id(&mut e) as i32;
                e.images.insert(h, (width, height, rgba));
                e.avatars.insert((user, size), h);
                Some((h, width, height))
            }
            _ => {
                x().avatars.insert((user, size), 0);
                None
            }
        };
        if let Some((h, w, hh)) = handle {
            let mut b = Bytes::new(24);
            b.u64(0, user).i32(8, h).i32(12, w as i32).i32(16, hh as i32);
            lock().post(AVATAR_IMAGE_LOADED, b.0);
        }
    });
    loading
}
extern "C" fn avatar_small(_t: This, user: u64) -> i32 {
    avatar(user, 0, 0)
}
extern "C" fn avatar_medium(_t: This, user: u64) -> i32 {
    avatar(user, 1, 0)
}
/// -1 while loading, like Steam.
extern "C" fn avatar_large(_t: This, user: u64) -> i32 {
    avatar(user, 2, -1)
}
extern "C" fn image_size(_t: This, image: i32, w: *mut u32, h: *mut u32) -> bool {
    match x().images.get(&image) {
        Some((iw, ih, _)) => {
            put(w, *iw);
            put(h, *ih);
            true
        }
        None => false,
    }
}
extern "C" fn image_rgba(_t: This, image: i32, buf: *mut u8, size: i32) -> bool {
    let e = x();
    let Some((_, _, rgba)) = e.images.get(&image) else { return false };
    if buf.is_null() || (size.max(0) as usize) < rgba.len() {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), buf, rgba.len()) };
    true
}

// ----------------------------------------------------------- ISteamHTTP

fn http_imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "CreateHTTPRequest" => f!(http_create),
        "SetHTTPRequestContextValue" => f!(http_context),
        "SetHTTPRequestNetworkActivityTimeout" => f!(http_timeout_secs),
        "SetHTTPRequestAbsoluteTimeoutMS" => f!(http_timeout_ms),
        "SetHTTPRequestHeaderValue" => f!(http_header),
        "SetHTTPRequestGetOrPostParameter" => f!(http_param),
        "SetHTTPRequestRawPostBody" => f!(http_body),
        "SendHTTPRequest" | "SendHTTPRequestAndStreamResponse" => f!(http_send),
        "GetHTTPResponseHeaderSize" => f!(http_header_size),
        "GetHTTPResponseHeaderValue" => f!(http_header_value),
        "GetHTTPResponseBodySize" => f!(http_body_size),
        "GetHTTPResponseBodyData" => f!(http_body_data),
        "GetHTTPStreamingResponseBodyData" => f!(http_body_part),
        "ReleaseHTTPRequest" => f!(http_release),
        "GetHTTPDownloadProgressPct" => f!(http_progress),
        "GetHTTPRequestWasTimedOut" => f!(http_timed_out),
        "SetHTTPRequestUserAgentInfo" | "SetHTTPRequestRequiresVerifiedCertificate" | "DeferHTTPRequest" | "PrioritizeHTTPRequest" | "SetHTTPRequestCookieContainer" => f!(http_ok),
        _ => None,
    }
}

fn with_http<R>(h: u32, f: impl FnOnce(&mut Http) -> R) -> Option<R> {
    x().http.get_mut(&h).map(f)
}

extern "C" fn http_create(_t: This, method: i32, url: *const c_char) -> u32 {
    let url = unsafe { text(url) };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return 0;
    }
    let mut e = x();
    let h = next_id(&mut e) as u32;
    e.http.insert(h, Http { method, url, ..Default::default() });
    h
}
extern "C" fn http_context(_t: This, h: u32, ctx: u64) -> bool {
    with_http(h, |r| r.context = ctx).is_some()
}
extern "C" fn http_timeout_secs(_t: This, h: u32, secs: u32) -> bool {
    with_http(h, |r| r.timeout_ms = secs.saturating_mul(1000)).is_some()
}
extern "C" fn http_timeout_ms(_t: This, h: u32, ms: u32) -> bool {
    with_http(h, |r| r.timeout_ms = ms).is_some()
}
extern "C" fn http_header(_t: This, h: u32, name: *const c_char, value: *const c_char) -> bool {
    let (n, v) = unsafe { (text(name), text(value)) };
    with_http(h, |r| r.headers.push((n, v))).is_some()
}
extern "C" fn http_param(_t: This, h: u32, name: *const c_char, value: *const c_char) -> bool {
    let (n, v) = unsafe { (text(name), text(value)) };
    with_http(h, |r| r.params.push((n, v))).is_some()
}
extern "C" fn http_body(_t: This, h: u32, content_type: *const c_char, body: *const u8, len: u32) -> bool {
    let ct = unsafe { text(content_type) };
    let data = slice(body, len as usize).to_vec();
    with_http(h, |r| r.body = Some((ct, data))).is_some()
}
extern "C" fn http_ok(_t: This) -> bool {
    true
}
extern "C" fn http_send(_t: This, h: u32, call_out: *mut u64) -> bool {
    let Some(req) = with_http(h, |r| Request::Http { method: r.method, url: r.url.clone(), headers: r.headers.clone(), params: r.params.clone(), body: r.body.clone(), timeout_ms: r.timeout_ms }) else {
        return false;
    };
    let ctx = with_http(h, |r| r.context).unwrap_or(0);
    let call = lock().new_call();
    put(call_out, call);
    std::thread::spawn(move || {
        let (status, headers, body, timed_out) = match crate::ipc::request(req) {
            Some(Reply::HttpDone { status, headers, body, timed_out }) => (status, headers, body, timed_out),
            _ => (0, Vec::new(), Vec::new(), false),
        };
        let size = body.len() as u32;
        with_http(h, |r| r.response = Some((status, headers, body, timed_out)));
        // HTTPRequestCompleted_t: request, context, success, status, body size.
        let mut b = Bytes::new(32);
        b.u32(0, h).u64(8, ctx).u8(16, (status != 0) as u8).i32(20, status).u32(24, size);
        let mut g = lock();
        g.complete(call, HTTP_REQUEST_COMPLETED, b.0.clone(), false);
        g.post(HTTP_REQUEST_COMPLETED, b.0);
    });
    true
}
extern "C" fn http_header_size(_t: This, h: u32, name: *const c_char, size: *mut u32) -> bool {
    let n = unsafe { text(name) };
    let v = with_http(h, |r| r.response.as_ref().and_then(|(_, hs, _, _)| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(&n)).map(|(_, v)| v.len() + 1))).flatten();
    match v {
        Some(len) => {
            put(size, len as u32);
            true
        }
        None => false,
    }
}
extern "C" fn http_header_value(_t: This, h: u32, name: *const c_char, buf: *mut c_char, size: u32) -> bool {
    let n = unsafe { text(name) };
    let v = with_http(h, |r| r.response.as_ref().and_then(|(_, hs, _, _)| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(&n)).map(|(_, v)| v.clone()))).flatten();
    match v {
        Some(v) => copy_out(&v, buf, size as usize) > 0,
        None => false,
    }
}
extern "C" fn http_body_size(_t: This, h: u32, size: *mut u32) -> bool {
    match with_http(h, |r| r.response.as_ref().map(|(_, _, b, _)| b.len() as u32)).flatten() {
        Some(n) => {
            put(size, n);
            true
        }
        None => false,
    }
}
fn copy_body(h: u32, offset: u32, buf: *mut u8, size: u32) -> bool {
    with_http(h, |r| {
        let Some((_, _, body, _)) = &r.response else { return false };
        let start = offset as usize;
        let end = start + size as usize;
        if buf.is_null() || end > body.len() {
            return false;
        }
        unsafe { std::ptr::copy_nonoverlapping(body[start..end].as_ptr(), buf, size as usize) };
        true
    })
    .unwrap_or(false)
}
extern "C" fn http_body_data(_t: This, h: u32, buf: *mut u8, size: u32) -> bool {
    copy_body(h, 0, buf, size)
}
extern "C" fn http_body_part(_t: This, h: u32, offset: u32, buf: *mut u8, size: u32) -> bool {
    copy_body(h, offset, buf, size)
}
extern "C" fn http_release(_t: This, h: u32) -> bool {
    x().http.remove(&h).is_some()
}
extern "C" fn http_progress(_t: This, h: u32, pct: *mut f32) -> bool {
    match with_http(h, |r| r.response.is_some()) {
        Some(done) => {
            put(pct, if done { 100.0 } else { 0.0 });
            true
        }
        None => false,
    }
}
extern "C" fn http_timed_out(_t: This, h: u32, out: *mut bool) -> bool {
    match with_http(h, |r| r.response.as_ref().map(|x| x.3).unwrap_or(false)) {
        Some(t) => {
            put(out, t);
            true
        }
        None => false,
    }
}

// ------------------------------------------- installed Workshop items

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaderboard_entry_layout() {
        x().entry_sets.insert(99, vec![LbEntry { steam_id: 7, rank: 1, score: 500, details: vec![3, 4], ugc: u64::MAX }]);
        let mut out = [0u8; 32];
        let mut details = [0i32; 4];
        assert!(lb_entry(std::ptr::null_mut(), 99, 0, out.as_mut_ptr(), details.as_mut_ptr(), 4));
        assert_eq!(u64::from_le_bytes(out[..8].try_into().unwrap()), 7);
        assert_eq!(i32::from_le_bytes(out[12..16].try_into().unwrap()), 500);
        assert_eq!(i32::from_le_bytes(out[16..20].try_into().unwrap()), 2);
        assert_eq!(&details[..2], &[3, 4]);
        assert!(!lb_entry(std::ptr::null_mut(), 99, 1, out.as_mut_ptr(), details.as_mut_ptr(), 4));
    }
}

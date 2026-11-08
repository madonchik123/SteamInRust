//! ISteamUGC: the Workshop for games. Queries, subscriptions, votes and
//! favorites go to Steam's PublishedFile service through SteamInRust;
//! items download into `steamapps/workshop/content/<app>/<item>` like
//! Steam's client puts them, and subscribed items that aren't there yet
//! download when the game first asks for its subscriptions.
//!
//! Publishing items (CreateItem / SubmitItemUpdate) isn't supported: it
//! needs Steam's UGC upload path. Those calls fail with k_EResultFail.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, copy_out, put, slice, text};
use crate::protocol::{Reply, Request, UgcItem, UgcQuery};
use crate::state::{Bytes, lock};
use crate::vt::Fp;
use std::collections::{HashMap, HashSet};
use std::ffi::c_char;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

const QUERY_COMPLETED: i32 = 3401;
const REQUEST_DETAILS_RESULT: i32 = 3402;
const CREATE_ITEM_RESULT: i32 = 3403;
const SUBMIT_ITEM_UPDATE_RESULT: i32 = 3404;
const ITEM_INSTALLED: i32 = 3405;
const DOWNLOAD_ITEM_RESULT: i32 = 3406;
const FAVORITES_CHANGED: i32 = 3407;
const SET_VOTE_RESULT: i32 = 3408;
const GET_VOTE_RESULT: i32 = 3409;
const START_PLAYTIME_RESULT: i32 = 3410;
const STOP_PLAYTIME_RESULT: i32 = 3411;
const SUBSCRIBE_RESULT: i32 = 1313;
const UNSUBSCRIBE_RESULT: i32 = 1315;
const FILE_SUBSCRIBED: i32 = 1321;
const FILE_UNSUBSCRIBED: i32 = 1322;

const INVALID: u64 = u64::MAX;
const OK: i32 = 1;
const FAIL: i32 = 2;

/// SteamUGCDetails_t up to m_unNumChildren: the part every SDK version has
/// (newer ones append m_ulTotalFilesSize, which old games have no room for).
const DETAILS: usize = 9772;
const DETAILS_PADDED: usize = 9776;

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "CreateQueryUserUGCRequest" => f!(create_user),
        "CreateQueryAllUGCRequestPage" | "CreateQueryAllUGCRequest" => f!(create_all_page),
        "CreateQueryAllUGCRequestCursor" => f!(create_all_cursor),
        "CreateQueryUGCDetailsRequest" => f!(create_details),
        "SendQueryUGCRequest" => f!(send_query),
        "GetQueryUGCResult" => f!(result),
        "GetQueryUGCNumTags" => f!(num_tags),
        "GetQueryUGCTag" => f!(tag),
        "GetQueryUGCTagDisplayName" => f!(tag_display),
        "GetQueryUGCPreviewURL" => f!(preview_url),
        "GetQueryUGCMetadata" => f!(metadata),
        "GetQueryUGCChildren" => f!(children),
        "GetQueryUGCStatistic" => f!(statistic),
        "GetQueryUGCNumAdditionalPreviews" => f!(num_previews),
        "GetQueryUGCAdditionalPreview" => f!(preview),
        "GetQueryUGCNumKeyValueTags" => f!(num_kv),
        "GetQueryUGCKeyValueTag" => f!(kv),
        "GetQueryFirstUGCKeyValueTag" => f!(first_kv),
        "ReleaseQueryUGCRequest" => f!(release),
        "AddRequiredTag" => f!(add_required_tag),
        "AddRequiredTagGroup" => f!(add_tag_group),
        "AddExcludedTag" => f!(add_excluded_tag),
        "SetReturnOnlyIDs" => f!(only_ids),
        "SetReturnKeyValueTags" => f!(return_kv),
        "SetReturnLongDescription" => f!(long_description),
        "SetReturnMetadata" => f!(return_metadata),
        "SetReturnChildren" => f!(return_children),
        "SetReturnAdditionalPreviews" => f!(return_previews),
        "SetReturnTotalOnly" => f!(total_only),
        "SetReturnPlaytimeStats" => f!(playtime_days),
        "SetLanguage" => f!(language),
        "SetAllowCachedResponse" => f!(max_age),
        "SetCloudFileNameFilter" => f!(cloud_filename),
        "SetMatchAnyTag" => f!(match_any),
        "SetSearchText" => f!(search),
        "SetRankedByTrendDays" => f!(trend_days),
        "SetTimeCreatedDateRange" => f!(created_range),
        "SetTimeUpdatedDateRange" => f!(updated_range),
        "AddRequiredKeyValueTag" => f!(add_kv),
        "SetAdminQuery" => f!(yes),
        "RequestUGCDetails" => f!(request_details),
        "CreateItem" => f!(create_item),
        "StartItemUpdate" => f!(start_update),
        "SubmitItemUpdate" => f!(submit_update),
        "SetUserItemVote" => f!(set_vote),
        "GetUserItemVote" => f!(get_vote),
        "AddItemToFavorites" => f!(favorite_add),
        "RemoveItemFromFavorites" => f!(favorite_remove),
        "SubscribeItem" => f!(subscribe),
        "UnsubscribeItem" => f!(unsubscribe),
        "GetNumSubscribedItems" => f!(count),
        "GetSubscribedItems" => f!(items),
        "GetItemState" => f!(state),
        "GetItemInstallInfo" => f!(install_info),
        "GetItemDownloadInfo" => f!(download_info),
        "DownloadItem" => f!(download),
        "BInitWorkshopForGameServer" => f!(yes),
        "StartPlaytimeTracking" => f!(start_playtime),
        "StopPlaytimeTracking" | "StopPlaytimeTrackingForAllItems" => f!(stop_playtime),
        _ => None,
    }
}

#[derive(Default)]
struct Ugc {
    next: u64,
    queries: HashMap<u64, (UgcQuery, Vec<UgcItem>)>,
    subscribed: Option<Vec<u64>>,
    /// Downloading now: item -> expected bytes.
    downloading: HashMap<u64, u64>,
    queued: HashSet<u64>,
}

static U: LazyLock<Mutex<Ugc>> = LazyLock::new(|| Mutex::new(Ugc { next: 1, ..Default::default() }));

/// Lock order: this first, then the global state.
fn u() -> MutexGuard<'static, Ugc> {
    U.lock().unwrap_or_else(|e| e.into_inner())
}

extern "C" fn yes() -> bool {
    true
}

fn app_id() -> u32 {
    lock().session.as_ref().map(|s| s.app_id).unwrap_or(0)
}

// --------------------------------------------------------------- queries

fn new_query(q: UgcQuery) -> u64 {
    let mut u = u();
    let h = u.next;
    u.next += 1;
    let language = lock().session.as_ref().map(|s| s.game_language.clone()).unwrap_or_default();
    u.queries.insert(h, (UgcQuery { language, ..q }, Vec::new()));
    h
}

/// Change a query that hasn't been sent; false for a bad handle.
fn with_query(h: u64, f: impl FnOnce(&mut UgcQuery)) -> bool {
    match u().queries.get_mut(&h) {
        Some((q, _)) => {
            f(q);
            true
        }
        None => false,
    }
}

extern "C" fn create_user(_t: This, account: u32, list: i32, matching: i32, sort: i32, creator: u32, consumer: u32, page: u32) -> u64 {
    new_query(UgcQuery { kind: 0, account, list, matching, sort, creator_app: creator, consumer_app: consumer, page, ..Default::default() })
}

extern "C" fn create_all_page(_t: This, query: i32, matching: i32, creator: u32, consumer: u32, page: u32) -> u64 {
    new_query(UgcQuery { kind: 1, query, matching, creator_app: creator, consumer_app: consumer, page, ..Default::default() })
}

extern "C" fn create_all_cursor(_t: This, query: i32, matching: i32, creator: u32, consumer: u32, cursor: *const c_char) -> u64 {
    let c = unsafe { text(cursor) };
    // "*" (or empty) means the first page.
    let cursor = Some(if c.is_empty() { "*".to_string() } else { c });
    new_query(UgcQuery { kind: 1, query, matching, creator_app: creator, consumer_app: consumer, page: 1, cursor, ..Default::default() })
}

extern "C" fn create_details(_t: This, ids: *const u64, n: u32) -> u64 {
    if ids.is_null() || n == 0 {
        return INVALID;
    }
    let ids = (0..n as usize).map(|i| unsafe { ids.add(i).read_unaligned() }).collect();
    new_query(UgcQuery { kind: 2, ids, ..Default::default() })
}

/// SteamUGCQueryCompleted_t: handle, EResult, returned, total, cached, next cursor.
fn query_completed(h: u64, eresult: i32, returned: u32, total: u32, cursor: &str) -> Vec<u8> {
    let mut b = Bytes::new(280);
    b.u64(0, h).i32(8, eresult).u32(12, returned).u32(16, total).str(21, 256, cursor);
    b.0
}

extern "C" fn send_query(_t: This, h: u64) -> u64 {
    let Some(q) = u().queries.get(&h).map(|(q, _)| q.clone()) else { return 0 };
    let call = lock().new_call();
    std::thread::spawn(move || {
        let (eresult, total, items, cursor) = match crate::ipc::request(Request::UgcQuery(Box::new(q))) {
            Some(Reply::UgcResults { eresult, total, items, cursor }) => (eresult, total, items, cursor),
            _ => (FAIL, 0, Vec::new(), String::new()),
        };
        let n = items.len() as u32;
        if let Some(e) = u().queries.get_mut(&h) {
            e.1 = items;
        }
        crate::log!("ugc: query {h}: result {eresult}, {n} of {total}");
        lock().complete(call, QUERY_COMPLETED, query_completed(h, eresult, n, total, &cursor), eresult != OK);
    });
    call
}

/// A result of a finished query.
fn item_of(h: u64, i: u32) -> Option<UgcItem> {
    u().queries.get(&h)?.1.get(i as usize).cloned()
}

/// SteamUGCDetails_t (Windows packing).
fn details(it: &UgcItem) -> Vec<u8> {
    let mut b = Bytes::new(DETAILS_PADDED);
    let tags: Vec<&str> = it.tags.iter().map(|(t, _)| t.as_str()).collect();
    b.u64(0, it.id).i32(8, it.result).i32(12, it.file_type).u32(16, it.creator_app).u32(20, it.consumer_app);
    b.str(24, 129, &it.title).str(153, 8000, &it.description);
    b.u64(8160, it.owner).u32(8168, it.created).u32(8172, it.updated).u32(8176, it.created).i32(8180, it.visibility);
    b.u8(8184, it.banned as u8).u8(8185, it.accepted as u8).u8(8186, (tags.join(",").len() > 1024) as u8).str(8187, 1025, &tags.join(","));
    b.u64(9216, it.file).u64(9224, it.preview_file).str(9232, 260, &it.file_name);
    b.i32(9492, it.file_size.min(i32::MAX as u64) as i32).i32(9496, it.preview_size.min(i32::MAX as u64) as i32).str(9500, 256, &it.url);
    b.u32(9756, it.votes_up).u32(9760, it.votes_down).u32(9764, it.score.to_bits()).u32(9768, it.children.len() as u32);
    b.0
}

fn write(out: *mut u8, data: &[u8]) {
    if !out.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len()) };
    }
}

extern "C" fn result(_t: This, h: u64, i: u32, out: *mut u8) -> bool {
    let Some(it) = item_of(h, i) else { return false };
    write(out, &details(&it)[..DETAILS]);
    true
}

extern "C" fn num_tags(_t: This, h: u64, i: u32) -> u32 {
    item_of(h, i).map(|it| it.tags.len() as u32).unwrap_or(0)
}

extern "C" fn tag(_t: This, h: u64, i: u32, tag: u32, buf: *mut c_char, len: u32) -> bool {
    let Some((t, _)) = item_of(h, i).and_then(|it| it.tags.get(tag as usize).cloned()) else { return false };
    copy_out(&t, buf, len as usize);
    true
}

extern "C" fn tag_display(_t: This, h: u64, i: u32, tag: u32, buf: *mut c_char, len: u32) -> bool {
    let Some((t, d)) = item_of(h, i).and_then(|it| it.tags.get(tag as usize).cloned()) else { return false };
    copy_out(if d.is_empty() { &t } else { &d }, buf, len as usize);
    true
}

extern "C" fn preview_url(_t: This, h: u64, i: u32, buf: *mut c_char, len: u32) -> bool {
    let Some(it) = item_of(h, i) else { return false };
    copy_out(&it.preview_url, buf, len as usize);
    true
}

extern "C" fn metadata(_t: This, h: u64, i: u32, buf: *mut c_char, len: u32) -> bool {
    let Some(it) = item_of(h, i) else { return false };
    copy_out(&it.metadata, buf, len as usize);
    true
}

extern "C" fn children(_t: This, h: u64, i: u32, out: *mut u64, max: u32) -> bool {
    let Some(it) = item_of(h, i) else { return false };
    for (k, id) in it.children.iter().take(max as usize).enumerate() {
        if !out.is_null() {
            unsafe { out.add(k).write_unaligned(*id) };
        }
    }
    true
}

extern "C" fn statistic(_t: This, h: u64, i: u32, stat: i32, out: *mut u64) -> bool {
    let Some(v) = item_of(h, i).and_then(|it| it.stats.iter().find(|(s, _)| *s == stat).map(|(_, v)| *v)) else { return false };
    put(out, v);
    true
}

extern "C" fn num_previews(_t: This, h: u64, i: u32) -> u32 {
    item_of(h, i).map(|it| it.previews.len() as u32).unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
extern "C" fn preview(_t: This, h: u64, i: u32, p: u32, url: *mut c_char, url_len: u32, name: *mut c_char, name_len: u32, kind: *mut i32) -> bool {
    let Some((u, t, n)) = item_of(h, i).and_then(|it| it.previews.get(p as usize).cloned()) else { return false };
    copy_out(&u, url, url_len as usize);
    copy_out(&n, name, name_len as usize);
    put(kind, t);
    true
}

extern "C" fn num_kv(_t: This, h: u64, i: u32) -> u32 {
    item_of(h, i).map(|it| it.kv.len() as u32).unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
extern "C" fn kv(_t: This, h: u64, i: u32, k: u32, key: *mut c_char, key_len: u32, value: *mut c_char, value_len: u32) -> bool {
    let Some((a, b)) = item_of(h, i).and_then(|it| it.kv.get(k as usize).cloned()) else { return false };
    copy_out(&a, key, key_len as usize);
    copy_out(&b, value, value_len as usize);
    true
}

extern "C" fn first_kv(_t: This, h: u64, i: u32, key: *const c_char, value: *mut c_char, value_len: u32) -> bool {
    let key = unsafe { text(key) };
    let Some(v) = item_of(h, i).and_then(|it| it.kv.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())) else { return false };
    copy_out(&v, value, value_len as usize);
    true
}

extern "C" fn release(_t: This, h: u64) -> bool {
    u().queries.remove(&h).is_some()
}

extern "C" fn add_required_tag(_t: This, h: u64, tag: *const c_char) -> bool {
    let t = unsafe { text(tag) };
    with_query(h, |q| q.required_tags.push(t))
}

/// SteamParamStringArray_t: const char** strings, int32 count.
fn string_array(p: *const u8) -> Vec<String> {
    if p.is_null() {
        return Vec::new();
    }
    let w = size_of::<usize>();
    let d = slice(p, w + 4);
    let list = usize::from_le_bytes(d[..w].try_into().unwrap()) as *const *const c_char;
    let n = i32::from_le_bytes(d[w..w + 4].try_into().unwrap()).clamp(0, 1024) as usize;
    if list.is_null() {
        return Vec::new();
    }
    (0..n).map(|i| unsafe { text(*list.add(i)) }).collect()
}

extern "C" fn add_tag_group(_t: This, h: u64, tags: *const u8) -> bool {
    let g = string_array(tags);
    with_query(h, |q| q.tag_groups.push(g))
}

extern "C" fn add_excluded_tag(_t: This, h: u64, tag: *const c_char) -> bool {
    let t = unsafe { text(tag) };
    with_query(h, |q| q.excluded_tags.push(t))
}

macro_rules! flag {
    ($name:ident, $field:ident) => {
        extern "C" fn $name(_t: This, h: u64, on: bool) -> bool {
            with_query(h, |q| q.$field = on)
        }
    };
}
flag!(only_ids, only_ids);
flag!(return_kv, kv_tags);
flag!(long_description, long_description);
flag!(return_metadata, metadata);
flag!(return_children, children);
flag!(return_previews, previews);
flag!(total_only, total_only);
flag!(match_any, match_any);

extern "C" fn playtime_days(_t: This, h: u64, days: u32) -> bool {
    with_query(h, |q| q.playtime_days = days)
}

extern "C" fn language(_t: This, h: u64, l: *const c_char) -> bool {
    let l = unsafe { text(l) };
    with_query(h, |q| q.language = l)
}

extern "C" fn max_age(_t: This, h: u64, seconds: u32) -> bool {
    with_query(h, |q| q.max_age = seconds)
}

extern "C" fn cloud_filename(_t: This, h: u64, name: *const c_char) -> bool {
    let n = unsafe { text(name) };
    with_query(h, |q| q.cloud_filename = n)
}

extern "C" fn search(_t: This, h: u64, s: *const c_char) -> bool {
    let s = unsafe { text(s) };
    with_query(h, |q| q.search = s)
}

extern "C" fn trend_days(_t: This, h: u64, days: u32) -> bool {
    with_query(h, |q| q.trend_days = days)
}

extern "C" fn created_range(_t: This, h: u64, start: u32, end: u32) -> bool {
    with_query(h, |q| q.created = (start, end))
}

extern "C" fn updated_range(_t: This, h: u64, start: u32, end: u32) -> bool {
    with_query(h, |q| q.updated = (start, end))
}

extern "C" fn add_kv(_t: This, h: u64, key: *const c_char, value: *const c_char) -> bool {
    let (k, v) = unsafe { (text(key), text(value)) };
    with_query(h, |q| q.kv.push((k, v)))
}

/// SteamUGCRequestUGCDetailsResult_t: details (the common layout) then "cached".
extern "C" fn request_details(_t: This, id: u64, _max_age: u32) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let item = match crate::ipc::request(Request::UgcQuery(Box::new(UgcQuery { kind: 2, ids: vec![id], long_description: true, ..Default::default() }))) {
            Some(Reply::UgcResults { items, .. }) => items.into_iter().next(),
            _ => None,
        };
        let mut b = details(&item.clone().unwrap_or(UgcItem { id, result: FAIL, ..Default::default() }));
        b.extend_from_slice(&[0u8; 8]);
        lock().complete(call, REQUEST_DETAILS_RESULT, b, item.is_none());
    });
    call
}

// ------------------------------------------------------------ publishing

extern "C" fn create_item(_t: This, _app: u32, _kind: i32) -> u64 {
    crate::log!("ugc: publishing items isn't supported");
    let mut b = Bytes::new(24);
    b.i32(0, FAIL);
    let mut g = lock();
    let call = g.new_call();
    g.complete(call, CREATE_ITEM_RESULT, b.0, false);
    call
}

extern "C" fn start_update(_t: This, _app: u32, _id: u64) -> u64 {
    INVALID
}

extern "C" fn submit_update(_t: This, _h: u64, _note: *const c_char) -> u64 {
    let mut b = Bytes::new(16);
    b.i32(0, FAIL);
    let mut g = lock();
    let call = g.new_call();
    g.complete(call, SUBMIT_ITEM_UPDATE_RESULT, b.0, false);
    call
}

// --------------------------------------------------- votes and favorites

/// Run `req` in the background and finish an API call with `make(reply)`.
fn call(req: Request, callback: i32, make: impl FnOnce(Option<Reply>) -> Vec<u8> + Send + 'static) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let data = make(crate::ipc::request(req));
        lock().complete(call, callback, data, false);
    });
    call
}

fn eresult(r: &Option<Reply>) -> i32 {
    match r {
        Some(Reply::Result { eresult }) => *eresult,
        _ => FAIL,
    }
}

extern "C" fn set_vote(_t: This, id: u64, up: bool) -> u64 {
    call(Request::UgcVote { id, up }, SET_VOTE_RESULT, move |r| {
        let mut b = Bytes::new(16);
        b.u64(0, id).i32(8, eresult(&r)).u8(12, up as u8);
        b.0
    })
}

extern "C" fn get_vote(_t: This, id: u64) -> u64 {
    call(Request::UgcGetVote { id }, GET_VOTE_RESULT, move |r| {
        let mut b = Bytes::new(16);
        match r {
            Some(Reply::UgcVote { eresult, up, down, skipped }) => b.u64(0, id).i32(8, eresult).u8(12, up as u8).u8(13, down as u8).u8(14, skipped as u8),
            _ => b.u64(0, id).i32(8, FAIL),
        };
        b.0
    })
}

fn favorite(id: u64, on: bool) -> u64 {
    call(Request::UgcSubscribe { id, list: 2, on }, FAVORITES_CHANGED, move |r| {
        let mut b = Bytes::new(16);
        b.u64(0, id).i32(8, eresult(&r)).u8(12, on as u8);
        b.0
    })
}

extern "C" fn favorite_add(_t: This, _app: u32, id: u64) -> u64 {
    favorite(id, true)
}

extern "C" fn favorite_remove(_t: This, _app: u32, id: u64) -> u64 {
    favorite(id, false)
}

// ----------------------------------------------------------- subscriptions

/// RemoteStorage(Un)SubscribePublishedFileResult_t: EResult, item.
fn subscription(id: u64, on: bool) -> u64 {
    call(Request::UgcSubscribe { id, list: 1, on }, if on { SUBSCRIBE_RESULT } else { UNSUBSCRIBE_RESULT }, move |r| {
        let result = eresult(&r);
        if result == OK {
            if let Some(list) = u().subscribed.as_mut() {
                list.retain(|x| *x != id);
                if on {
                    list.push(id);
                }
            }
            let mut b = Bytes::new(16);
            b.u64(0, id).u32(8, app_id());
            lock().post(if on { FILE_SUBSCRIBED } else { FILE_UNSUBSCRIBED }, b.0);
            // Steam downloads what you subscribe to.
            if on && !installed(id) {
                start_download(id);
            }
        }
        let mut b = Bytes::new(16);
        b.i32(0, result).u64(8, id);
        b.0
    })
}

extern "C" fn subscribe(_t: This, id: u64) -> u64 {
    subscription(id, true)
}

extern "C" fn unsubscribe(_t: This, id: u64) -> u64 {
    subscription(id, false)
}

/// The subscriptions (loaded once; offline: what's installed).
fn subscribed() -> Vec<u64> {
    if let Some(list) = u().subscribed.clone() {
        return list;
    }
    let list = match crate::ipc::request(Request::UgcSubscribed) {
        Some(Reply::UgcIds { ids }) => ids,
        _ => installed_items(),
    };
    crate::log!("ugc: {} subscribed item(s)", list.len());
    u().subscribed = Some(list.clone());
    // Like Steam: subscribed items the game can't find yet get downloaded.
    for id in list.iter().filter(|id| !installed(**id)) {
        start_download(*id);
    }
    list
}

extern "C" fn count(_t: This) -> u32 {
    subscribed().len() as u32
}

extern "C" fn items(_t: This, out: *mut u64, max: u32) -> u32 {
    let list = subscribed();
    let n = list.len().min(max as usize);
    for (i, id) in list.iter().take(n).enumerate() {
        if !out.is_null() {
            unsafe { out.add(i).write_unaligned(*id) };
        }
    }
    n as u32
}

/// EItemState: 1 subscribed, 4 installed, 16 downloading, 32 download pending.
extern "C" fn state(_t: This, id: u64) -> u32 {
    let sub = subscribed().contains(&id);
    let (downloading, pending) = {
        let u = u();
        (u.downloading.contains_key(&id), u.queued.contains(&id))
    };
    (sub as u32) | if installed(id) { 4 } else { 0 } | if downloading { 16 } else { 0 } | if pending && !downloading { 32 } else { 0 }
}

// ---------------------------------------------------------------- content

fn workshop_dir() -> Option<PathBuf> {
    let g = lock();
    let s = g.session.as_ref()?;
    let dir = PathBuf::from(&s.install_dir);
    let steamapps = dir.parent()?.parent()?;
    Some(steamapps.join("workshop").join("content").join(s.app_id.to_string()))
}

fn installed_items() -> Vec<u64> {
    let Some(dir) = workshop_dir() else { return Vec::new() };
    let mut out: Vec<u64> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default();
    out.sort_unstable();
    out
}

fn item_dir(id: u64) -> Option<PathBuf> {
    workshop_dir().map(|d| d.join(id.to_string()))
}

/// Installed: the folder exists and has something in it (and isn't downloading).
fn installed(id: u64) -> bool {
    if u().downloading.contains_key(&id) {
        return false;
    }
    item_dir(id).and_then(|d| std::fs::read_dir(d).ok()).is_some_and(|mut rd| rd.next().is_some())
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()).unwrap_or(0)
}

extern "C" fn install_info(_t: This, id: u64, size: *mut u64, folder: *mut c_char, folder_size: u32, stamp: *mut u32) -> bool {
    if !installed(id) {
        return false;
    }
    let Some(dir) = item_dir(id) else { return false };
    put(size, dir_size(&dir));
    copy_out(&dir.to_string_lossy(), folder, folder_size as usize);
    let t = std::fs::metadata(&dir).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as u32).unwrap_or(0);
    put(stamp, t);
    true
}

extern "C" fn download_info(_t: This, id: u64, done: *mut u64, total: *mut u64) -> bool {
    let Some(expected) = u().downloading.get(&id).copied() else { return false };
    let have = item_dir(id).map(|d| dir_size(&d)).unwrap_or(0);
    put(done, have.min(expected));
    put(total, expected);
    true
}

/// Download in the background; DownloadItemResult_t and ItemInstalled_t after.
fn start_download(id: u64) -> bool {
    {
        let mut u = u();
        if u.downloading.contains_key(&id) || !u.queued.insert(id) {
            return true;
        }
    }
    std::thread::spawn(move || {
        // One at a time, like Steam's download queue.
        static ONE: Mutex<()> = Mutex::new(());
        let _turn = ONE.lock().unwrap_or_else(|e| e.into_inner());
        u().downloading.insert(id, 0);
        crate::log!("ugc: downloading item {id}");
        let r = crate::ipc::request(Request::UgcDownload { id });
        {
            let mut u = u();
            u.downloading.remove(&id);
            u.queued.remove(&id);
        }
        let app = app_id();
        let result = match &r {
            Some(Reply::UgcDownloaded { eresult, .. }) => *eresult,
            _ => FAIL,
        };
        crate::log!("ugc: item {id} download result {result}");
        let mut b = Bytes::new(24);
        b.u32(0, app).u64(8, id).i32(16, result);
        let mut g = lock();
        g.post(DOWNLOAD_ITEM_RESULT, b.0);
        if result == OK {
            let mut b = Bytes::new(32);
            b.u32(0, app).u64(8, id);
            g.post(ITEM_INSTALLED, b.0);
        }
    });
    true
}

extern "C" fn download(_t: This, id: u64, _high_priority: bool) -> bool {
    id != 0 && start_download(id)
}

extern "C" fn start_playtime(_t: This, _ids: *const u64, _n: u32) -> u64 {
    let mut b = Bytes::new(4);
    b.i32(0, OK);
    let mut g = lock();
    let call = g.new_call();
    g.complete(call, START_PLAYTIME_RESULT, b.0, false);
    call
}

extern "C" fn stop_playtime(_t: This) -> u64 {
    let mut b = Bytes::new(4);
    b.i32(0, OK);
    let mut g = lock();
    let call = g.new_call();
    g.complete(call, STOP_PLAYTIME_RESULT, b.0, false);
    call
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_layout() {
        let it = UgcItem {
            id: 42,
            result: 1,
            title: "A map".into(),
            owner: 76561197960287930,
            tags: vec![("Maps".into(), String::new()), ("Co-op".into(), String::new())],
            file: 7,
            file_name: "map.bsp".into(),
            votes_up: 3,
            children: vec![1, 2],
            ..Default::default()
        };
        let d = details(&it);
        assert_eq!(d.len(), DETAILS_PADDED);
        assert_eq!(u64::from_le_bytes(d[0..8].try_into().unwrap()), 42);
        assert_eq!(&d[24..29], b"A map");
        assert_eq!(u64::from_le_bytes(d[8160..8168].try_into().unwrap()), 76561197960287930);
        assert_eq!(&d[8187..8197], b"Maps,Co-op");
        assert_eq!(u64::from_le_bytes(d[9216..9224].try_into().unwrap()), 7);
        assert_eq!(&d[9232..9239], b"map.bsp");
        assert_eq!(u32::from_le_bytes(d[9756..9760].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(d[9768..9772].try_into().unwrap()), 2);
        let c = query_completed(5, 1, 2, 10, "next");
        assert_eq!(c.len(), 280);
        assert_eq!(&c[21..25], b"next");
    }

    #[test]
    fn query_builder() {
        let h = create_all_page(std::ptr::null_mut(), 0, 0, 480, 480, 1);
        let tag = c"Maps";
        assert!(add_required_tag(std::ptr::null_mut(), h, tag.as_ptr()));
        assert!(only_ids(std::ptr::null_mut(), h, true));
        assert!(!only_ids(std::ptr::null_mut(), h + 1000, true));
        let q = u().queries[&h].0.clone();
        assert_eq!((q.kind, q.consumer_app, q.required_tags.as_slice(), q.only_ids), (1, 480, &["Maps".to_string()][..], true));
        assert!(release(std::ptr::null_mut(), h));
    }
}

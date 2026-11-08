//! A tiny "game" for testing the steamclient replacement: loads a real
//! `steam_api64.dll` (put it next to this exe), initialises it like a game
//! does and prints what Steam answers.
//!
//! `probe [--unlock ACH_NAME] [--stat NAME=VALUE] [--cloud]`
//! Writes only when asked (use with Spacewar, app 480).

use std::ffi::{CStr, CString, c_char, c_void};
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

struct Api(*mut c_void);

impl Api {
    fn f(&self, name: &str) -> Option<usize> {
        let n = CString::new(name).ok()?;
        let p = unsafe { GetProcAddress(self.0, n.as_ptr().cast()) } as usize;
        (p != 0).then_some(p)
    }
    /// First accessor that exists, e.g. SteamAPI_SteamUser_v023 / v021.
    fn iface(&self, base: &str, versions: &[&str]) -> usize {
        for v in versions {
            if let Some(f) = self.f(&format!("SteamAPI_{base}_{v}")) {
                let get: extern "C" fn() -> usize = unsafe { std::mem::transmute(f) };
                return get();
            }
        }
        0
    }
}

macro_rules! call {
    ($api:expr, $name:literal, fn($($t:ty),*) -> $r:ty, $($a:expr),*) => {{
        let f = $api.f($name).unwrap_or_else(|| panic!("{} missing", $name));
        let f: extern "C" fn($($t),*) -> $r = unsafe { std::mem::transmute(f) };
        f($($a),*)
    }};
}

#[repr(C)]
struct CallbackMsg {
    user: i32,
    callback: i32,
    param: *mut u8,
    size: i32,
}

fn s(p: *const c_char) -> String {
    if p.is_null() { "<null>".into() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// SteamNetworkingSockets / Messages through the real steam_api: a host
/// listens, connects to itself, accepts, and both sides exchange data.
fn sockets_test(api: &Api, pipe: i32, me: u64) {
    let sockets = api.iface("SteamNetworkingSockets_SteamAPI", &["v012", "v011", "v009"]);
    let messages = api.iface("SteamNetworkingMessages_SteamAPI", &["v002"]);
    let utils = api.iface("SteamNetworkingUtils_SteamAPI", &["v004", "v003"]);
    let mut status = [0u8; 272];
    println!("relay status  {}", call!(api, "SteamAPI_ISteamNetworkingUtils_GetRelayNetworkStatus", fn(usize, *mut u8) -> i32, utils, status.as_mut_ptr()));
    let mut id = [0u8; 136];
    id[0..4].copy_from_slice(&16i32.to_le_bytes());
    id[4..8].copy_from_slice(&8i32.to_le_bytes());
    id[8..16].copy_from_slice(&me.to_le_bytes());
    let listen = call!(api, "SteamAPI_ISteamNetworkingSockets_CreateListenSocketP2P", fn(usize, i32, i32, *const u8) -> u32, sockets, 7, 0, std::ptr::null());
    let conn = call!(api, "SteamAPI_ISteamNetworkingSockets_ConnectP2P", fn(usize, *const u8, i32, i32, *const u8) -> u32, sockets, id.as_ptr(), 7, 0, std::ptr::null());
    println!("listen {listen}, connect {conn}");
    let mut accepted = 0u32;
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        call!(api, "SteamAPI_ManualDispatch_RunFrame", fn(i32) -> (), pipe);
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        while call!(api, "SteamAPI_ManualDispatch_GetNextCallback", fn(i32, *mut CallbackMsg) -> bool, pipe, &mut msg) {
            if msg.callback == 1221 {
                let b = unsafe { std::slice::from_raw_parts(msg.param, msg.size as usize) };
                let h = u32::from_le_bytes(b[0..4].try_into().unwrap());
                let ls = u32::from_le_bytes(b[8 + 144..8 + 148].try_into().unwrap());
                let state = i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap());
                println!("  status: conn {h} listen {ls} state {state} ({} bytes)", msg.size);
                if ls == listen && state == 1 {
                    accepted = h;
                    println!("  AcceptConnection = {}", call!(api, "SteamAPI_ISteamNetworkingSockets_AcceptConnection", fn(usize, u32) -> i32, sockets, h));
                }
            } else {
                println!("  callback {}", msg.callback);
            }
            call!(api, "SteamAPI_ManualDispatch_FreeLastCallback", fn(i32) -> (), pipe);
        }
        if accepted != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let r = call!(api, "SteamAPI_ISteamNetworkingSockets_SendMessageToConnection", fn(usize, u32, *const u8, u32, i32, *mut i64) -> i32, sockets, conn, b"hello host".as_ptr(), 10, 8, std::ptr::null_mut());
    println!("SendMessageToConnection = {r}");
    let mut out = [std::ptr::null_mut::<u8>(); 4];
    let n = call!(api, "SteamAPI_ISteamNetworkingSockets_ReceiveMessagesOnConnection", fn(usize, u32, *mut *mut u8, i32) -> i32, sockets, accepted, out.as_mut_ptr(), 4);
    for m in &out[..n.max(0) as usize] {
        print_message(*m);
    }
    let r = call!(api, "SteamAPI_ISteamNetworkingMessages_SendMessageToUser", fn(usize, *const u8, *const u8, u32, i32, i32) -> i32, messages, id.as_ptr(), b"hi me".as_ptr(), 5, 8, 3);
    println!("SendMessageToUser = {r}");
    let n = call!(api, "SteamAPI_ISteamNetworkingMessages_ReceiveMessagesOnChannel", fn(usize, i32, *mut *mut u8, i32) -> i32, messages, 3, out.as_mut_ptr(), 4);
    for m in &out[..n.max(0) as usize] {
        print_message(*m);
    }
    println!("CloseConnection = {}", call!(api, "SteamAPI_ISteamNetworkingSockets_CloseConnection", fn(usize, u32, i32, *const c_char, bool) -> bool, sockets, conn, 0, std::ptr::null(), false));
}

/// Print a SteamNetworkingMessage_t, then release it like a game does.
fn print_message(m: *mut u8) {
    let w = size_of::<usize>();
    unsafe {
        let data = (m as *const *const u8).read_unaligned();
        let size = (m.add(w) as *const i32).read_unaligned();
        let conn = (m.add(w + 4) as *const u32).read_unaligned();
        let from = (m.add(w + 8 + 8) as *const u64).read_unaligned();
        let release_at = if w == 8 { 184 } else { 180 };
        let release: extern "C" fn(*mut u8) = std::mem::transmute((m.add(release_at) as *const usize).read_unaligned());
        println!("  message on conn {conn} from {from}: {:?}", String::from_utf8_lossy(std::slice::from_raw_parts(data, size as usize)));
        release(m);
    }
}

/// Vtable slot `i` of a C++ interface object.
fn slot(obj: usize, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

/// A SteamNetworkingMessage_t's connection, sender and bytes; released after.
fn take_message(m: *mut u8) -> (u32, u64, Vec<u8>) {
    let w = size_of::<usize>();
    unsafe {
        let data = (m as *const *const u8).read_unaligned();
        let size = (m.add(w) as *const i32).read_unaligned();
        let conn = (m.add(w + 4) as *const u32).read_unaligned();
        let from = (m.add(w + 8 + 8) as *const u64).read_unaligned();
        let bytes = std::slice::from_raw_parts(data, size as usize).to_vec();
        let release: extern "C" fn(*mut u8) = std::mem::transmute((m.add(if w == 8 { 184 } else { 180 }) as *const usize).read_unaligned());
        release(m);
        (conn, from, bytes)
    }
}

fn identity(steam_id: u64) -> [u8; 136] {
    let mut id = [0u8; 136];
    id[0..4].copy_from_slice(&16i32.to_le_bytes());
    id[4..8].copy_from_slice(&8i32.to_le_bytes());
    id[8..16].copy_from_slice(&steam_id.to_le_bytes());
    id
}

/// `--direct <steamclient.dll> --role host|join --peer <steam id>`: load our
/// DLL without steam_api and play one side of a two-player test (the client
/// crate's `p2p_pair` example plays Steam for both). 64-bit only.
fn direct(dll: &str, host: bool, peer: u64) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let who = if host { "host" } else { "join" };
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    let sockets = generic(client, user, pipe, c"SteamNetworkingSockets012".as_ptr());
    let messages = generic(client, user, pipe, c"SteamNetworkingMessages002".as_ptr());
    assert!(sockets != 0 && messages != 0);
    let get_callback: extern "C" fn(i32, *mut CallbackMsg) -> bool = unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) };
    let free_callback: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) };
    // ISteamNetworkingSockets012 / Messages002 slots.
    type Listen = extern "C" fn(usize, i32, i32, *const u8) -> u32;
    type Connect = extern "C" fn(usize, *const u8, i32, i32, *const u8) -> u32;
    type Accept = extern "C" fn(usize, u32) -> i32;
    type Close = extern "C" fn(usize, u32, i32, *const c_char, bool) -> bool;
    type Send = extern "C" fn(usize, u32, *const u8, u32, i32, *mut i64) -> i32;
    type Receive = extern "C" fn(usize, u32, *mut *mut u8, i32) -> i32;
    type SendUser = extern "C" fn(usize, *const u8, *const u8, u32, i32, i32) -> i32;
    type ReceiveChannel = extern "C" fn(usize, i32, *mut *mut u8, i32) -> i32;
    type AcceptUser = extern "C" fn(usize, *const u8) -> bool;
    let (listen, connect_p2p): (Listen, Connect) = unsafe { (std::mem::transmute(slot(sockets, 2)), std::mem::transmute(slot(sockets, 3))) };
    let (accept, close): (Accept, Close) = unsafe { (std::mem::transmute(slot(sockets, 4)), std::mem::transmute(slot(sockets, 5))) };
    let (send, receive): (Send, Receive) = unsafe { (std::mem::transmute(slot(sockets, 11)), std::mem::transmute(slot(sockets, 14))) };
    let (send_user, receive_channel): (SendUser, ReceiveChannel) = unsafe { (std::mem::transmute(slot(messages, 0)), std::mem::transmute(slot(messages, 1))) };
    let accept_user: AcceptUser = unsafe { std::mem::transmute(slot(messages, 2)) };

    let them = identity(peer);
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    let mut conn = 0u32;
    let mut passed: Vec<&str> = Vec::new();
    if host {
        let ls = listen(sockets, 7, 0, std::ptr::null());
        println!("[host] listening: socket {ls}");
    } else {
        conn = connect_p2p(sockets, them.as_ptr(), 7, 0, std::ptr::null());
        println!("[join] connecting: connection {conn}");
        println!("[join] SendMessageToUser = {}", send_user(messages, them.as_ptr(), b"msg hi".as_ptr(), 6, 8, 3));
    }
    // host: socket message + user message, then the joiner closes;
    // join: socket reply + big message + user reply.
    let want = if host { 2 } else { 3 };
    let until = Instant::now() + Duration::from_secs(40);
    let mut closed = false;
    while Instant::now() < until {
        if passed.len() >= want && (!host || closed) {
            break;
        }
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        while get_callback(pipe, &mut msg) {
            let b = unsafe { std::slice::from_raw_parts(msg.param, msg.size as usize) }.to_vec();
            free_callback(pipe);
            match msg.callback {
                1221 => {
                    let h = u32::from_le_bytes(b[0..4].try_into().unwrap());
                    let state = i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap());
                    let ls = u32::from_le_bytes(b[8 + 144..8 + 148].try_into().unwrap());
                    println!("[{who}] connection {h}: state {state}");
                    if host && ls != 0 && state == 1 {
                        println!("[host] AcceptConnection = {}", accept(sockets, h));
                    }
                    if state == 3 {
                        conn = h;
                        if !host {
                            println!("[join] send = {}", send(sockets, h, b"hello host".as_ptr(), 10, 8, std::ptr::null_mut()));
                        }
                    }
                    if state == 4 {
                        closed = true;
                        close(sockets, h, 0, std::ptr::null(), false);
                    }
                }
                1251 => {
                    let from = u64::from_le_bytes(b[8..16].try_into().unwrap());
                    println!("[{who}] session request from {from}: accept = {}", accept_user(messages, b.as_ptr()));
                }
                other => println!("[{who}] callback {other}"),
            }
        }
        let mut out = [std::ptr::null_mut::<u8>(); 8];
        if conn != 0 {
            let n = receive(sockets, conn, out.as_mut_ptr(), 8);
            for m in &out[..n.max(0) as usize] {
                let (c, from, data) = take_message(*m);
                if host && data == b"hello host" {
                    println!("[host] got {:?} on {c} from {from}", String::from_utf8_lossy(&data));
                    send(sockets, conn, b"hello join".as_ptr(), 10, 8, std::ptr::null_mut());
                    send(sockets, conn, big.as_ptr(), big.len() as u32, 8, std::ptr::null_mut());
                    passed.push("socket message");
                } else if !host && data == b"hello join" {
                    println!("[join] got {:?} from {from}", String::from_utf8_lossy(&data));
                    passed.push("socket reply");
                } else if !host && data == big {
                    println!("[join] got the 200 KB message intact");
                    passed.push("big message");
                } else {
                    println!("[{who}] unexpected {} bytes", data.len());
                }
            }
        }
        let n = receive_channel(messages, 3, out.as_mut_ptr(), 8);
        for m in &out[..n.max(0) as usize] {
            let (_, from, data) = take_message(*m);
            println!("[{who}] channel 3 from {from}: {:?}", String::from_utf8_lossy(&data));
            if host && data == b"msg hi" {
                send_user(messages, them.as_ptr(), b"msg back".as_ptr(), 8, 8, 3);
                passed.push("user message");
            }
            if !host && data == b"msg back" {
                passed.push("user reply");
            }
        }
        if !host && passed.len() >= want {
            println!("[join] closing");
            close(sockets, conn, 1000, c"done".as_ptr(), false);
            std::thread::sleep(Duration::from_millis(500));
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let ok = passed.len() >= want && (!host || closed);
    println!("[{who}] {} {passed:?}", if ok { "PASS" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}

/// `--direct <steamclient.dll> --test ugc`: Workshop through our DLL against
/// a signed-in SteamInRust (the client crate's `steamclient_attach`
/// example). Read-only on the account: queries, the subscription list, and
/// one item downloaded into the game's Workshop folder.
fn ugc_test(dll: &str) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    const V: &str = "STEAMUGC_INTERFACE_VERSION021";
    let ugc = generic(client, user, pipe, c"STEAMUGC_INTERFACE_VERSION021".as_ptr());
    assert!(ugc != 0);
    let m = |name: &str| slot(ugc, steamclient::slot_of(V, name).unwrap_or_else(|| panic!("{name}")));
    let get_callback: extern "C" fn(i32, *mut CallbackMsg) -> bool = unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) };
    let free_callback: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) };
    let call_result: extern "C" fn(i32, u64, *mut u8, i32, i32, *mut bool) -> bool = unsafe { std::mem::transmute(api.f("Steam_GetAPICallResult").unwrap()) };
    // Wait for a callback (or the API call `call` to finish); returns its bytes.
    let wait = |want: i32, call: u64, size: usize| -> Option<Vec<u8>> {
        let until = Instant::now() + Duration::from_secs(120);
        while Instant::now() < until {
            let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
            while get_callback(pipe, &mut msg) {
                let b = unsafe { std::slice::from_raw_parts(msg.param, msg.size as usize) }.to_vec();
                free_callback(pipe);
                if call != 0 && msg.callback == 703 && u64::from_le_bytes(b[0..8].try_into().unwrap()) == call {
                    let mut out = vec![0u8; size];
                    let mut failed = false;
                    call_result(pipe, call, out.as_mut_ptr(), size as i32, want, &mut failed);
                    return Some(out);
                }
                if call == 0 && msg.callback == want {
                    return Some(b);
                }
                println!("  (callback {})", msg.callback);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    };

    type Create = extern "C" fn(usize, i32, i32, u32, u32, u32) -> u64;
    type SetBool = extern "C" fn(usize, u64, bool) -> bool;
    type Send = extern "C" fn(usize, u64) -> u64;
    type GetResult = extern "C" fn(usize, u64, u32, *mut u8) -> bool;
    type GetString = extern "C" fn(usize, u64, u32, *mut c_char, u32) -> bool;
    type Statistic = extern "C" fn(usize, u64, u32, i32, *mut u64) -> bool;
    let create_all: Create = unsafe { std::mem::transmute(m("CreateQueryAllUGCRequestPage")) };
    let long_desc: SetBool = unsafe { std::mem::transmute(m("SetReturnLongDescription")) };
    let send: Send = unsafe { std::mem::transmute(m("SendQueryUGCRequest")) };
    let get: GetResult = unsafe { std::mem::transmute(m("GetQueryUGCResult")) };
    let preview: GetString = unsafe { std::mem::transmute(m("GetQueryUGCPreviewURL")) };
    let statistic: Statistic = unsafe { std::mem::transmute(m("GetQueryUGCStatistic")) };
    let num_tags: extern "C" fn(usize, u64, u32) -> u32 = unsafe { std::mem::transmute(m("GetQueryUGCNumTags")) };

    // Most subscribed items (EUGCQuery 9), items only.
    let q = create_all(ugc, 9, 0, 480, 480, 1);
    long_desc(ugc, q, true);
    let call = send(ugc, q);
    let done = wait(3401, call, 280).expect("query timed out");
    let (result, n, total) = (i32::from_le_bytes(done[8..12].try_into().unwrap()), u32::from_le_bytes(done[12..16].try_into().unwrap()), u32::from_le_bytes(done[16..20].try_into().unwrap()));
    println!("query: result {result}, {n} of {total} items");
    let mut pick: Option<(u64, i32)> = None;
    for i in 0..n {
        let mut d = vec![0u8; 9784];
        if !get(ugc, q, i, d.as_mut_ptr()) {
            continue;
        }
        let id = u64::from_le_bytes(d[0..8].try_into().unwrap());
        let title = s(d[24..].as_ptr().cast());
        let owner = u64::from_le_bytes(d[8160..8168].try_into().unwrap());
        let file = u64::from_le_bytes(d[9216..9224].try_into().unwrap());
        let size = i32::from_le_bytes(d[9492..9496].try_into().unwrap());
        let up = u32::from_le_bytes(d[9756..9760].try_into().unwrap());
        let mut url = [0 as c_char; 256];
        preview(ugc, q, i, url.as_mut_ptr(), 256);
        let mut subs = 0u64;
        statistic(ugc, q, i, 0, &mut subs);
        if i < 5 {
            println!("  #{i} {id} {title:?} by {owner}: {size} bytes, manifest {file}, {up} up, {subs} subscribers, {} tags, preview {}", num_tags(ugc, q, i), s(url.as_ptr()));
        }
        if file != 0 && size > 0 && pick.is_none_or(|(_, s)| size < s) {
            pick = Some((id, size));
        }
    }
    let count: extern "C" fn(usize, bool) -> u32 = unsafe { std::mem::transmute(m("GetNumSubscribedItems")) };
    println!("subscribed: {}", count(ugc, false));
    let Some((id, size)) = pick else {
        println!("no item with content to download");
        std::process::exit(if result == 1 { 0 } else { 1 });
    };
    let download: extern "C" fn(usize, u64, bool) -> bool = unsafe { std::mem::transmute(m("DownloadItem")) };
    let state: extern "C" fn(usize, u64) -> u32 = unsafe { std::mem::transmute(m("GetItemState")) };
    let info: extern "C" fn(usize, u64, *mut u64, *mut c_char, u32, *mut u32) -> bool = unsafe { std::mem::transmute(m("GetItemInstallInfo")) };
    println!("DownloadItem({id}, {size} bytes) = {}", download(ugc, id, true));
    let r = wait(3406, 0, 24).map(|b| i32::from_le_bytes(b[16..20].try_into().unwrap()));
    let mut folder = [0 as c_char; 512];
    let (mut on_disk, mut stamp) = (0u64, 0u32);
    let ok = info(ugc, id, &mut on_disk, folder.as_mut_ptr(), 512, &mut stamp);
    println!("download result {r:?}; state {}; install info {ok}: {} ({on_disk} bytes)", state(ugc, id), s(folder.as_ptr()));
    std::process::exit(if result == 1 && r == Some(1) && ok { 0 } else { 1 });
}

/// `--direct <steamclient.dll> --test inventory`: the player's items and the
/// game's item definitions through our DLL (read-only).
fn inventory_test(dll: &str) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    const V: &str = "STEAMINVENTORY_INTERFACE_V003";
    let inv = generic(client, user, pipe, c"STEAMINVENTORY_INTERFACE_V003".as_ptr());
    assert!(inv != 0);
    let m = |name: &str| slot(inv, steamclient::slot_of(V, name).unwrap_or_else(|| panic!("{name}")));
    let get_callback: extern "C" fn(i32, *mut CallbackMsg) -> bool = unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) };
    let free_callback: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) };
    let wait = |want: i32| -> Option<Vec<u8>> {
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
            while get_callback(pipe, &mut msg) {
                let b = unsafe { std::slice::from_raw_parts(msg.param, msg.size as usize) }.to_vec();
                free_callback(pipe);
                println!("  callback {}", msg.callback);
                if msg.callback == want {
                    return Some(b);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    };
    let all: extern "C" fn(usize, *mut i32) -> bool = unsafe { std::mem::transmute(m("GetAllItems")) };
    let status: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(m("GetResultStatus")) };
    let items: extern "C" fn(usize, i32, *mut u8, *mut u32) -> bool = unsafe { std::mem::transmute(m("GetResultItems")) };
    let mut h = 0i32;
    println!("GetAllItems = {} (handle {h})", all(inv, &mut h));
    wait(4700);
    let mut n = 0u32;
    items(inv, h, std::ptr::null_mut(), &mut n);
    let mut buf = vec![0u8; n as usize * 16];
    items(inv, h, buf.as_mut_ptr(), &mut n);
    println!("result {h}: status {}, {n} item(s)", status(inv, h));
    for c in buf.chunks(16).take(10) {
        println!("  item {} def {} x{}", u64::from_le_bytes(c[0..8].try_into().unwrap()), i32::from_le_bytes(c[8..12].try_into().unwrap()), u16::from_le_bytes(c[12..14].try_into().unwrap()));
    }
    let ids: extern "C" fn(usize, *mut i32, *mut u32) -> bool = unsafe { std::mem::transmute(m("GetItemDefinitionIDs")) };
    let prop: extern "C" fn(usize, i32, *const c_char, *mut c_char, *mut u32) -> bool = unsafe { std::mem::transmute(m("GetItemDefinitionProperty")) };
    let mut count = 0u32;
    if !ids(inv, std::ptr::null_mut(), &mut count) {
        wait(4702);
        ids(inv, std::ptr::null_mut(), &mut count);
    }
    let mut defs = vec![0i32; count as usize];
    ids(inv, defs.as_mut_ptr(), &mut count);
    println!("{count} item definition(s)");
    for d in defs.iter().take(5) {
        let mut name = [0 as c_char; 256];
        let mut size = 256u32;
        prop(inv, *d, c"name".as_ptr(), name.as_mut_ptr(), &mut size);
        let mut keys = [0 as c_char; 1024];
        let mut ksize = 1024u32;
        prop(inv, *d, std::ptr::null(), keys.as_mut_ptr(), &mut ksize);
        println!("  def {d}: {:?} [{}]", s(name.as_ptr()), s(keys.as_ptr()));
    }
    std::process::exit(if status(inv, h) == 1 && count > 0 { 0 } else { 1 });
}

/// A game's ISteamMatchmakingServerListResponse / Players / Rules objects:
/// a vtable of our functions, counting what arrives.
#[repr(C)]
struct Listener {
    vtable: *const usize,
}

static RESPONDED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static FAILED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static COMPLETE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static PLAYERS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static RULES: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static COUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

extern "C" fn on_responded(_this: usize, _req: usize, _i: i32) {
    RESPONDED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_failed(_this: usize, _req: usize, _i: i32) {
    FAILED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_complete(_this: usize, _req: usize, r: i32) {
    COMPLETE.store(r, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_player(_this: usize, name: *const c_char, score: i32, time: f32) {
    if COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 3 {
        println!("    player {:?} score {score} for {time:.0} s", s(name));
    }
}
extern "C" fn on_rule(_this: usize, k: *const c_char, v: *const c_char) {
    if COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 3 {
        println!("    rule {} = {}", s(k), s(v));
    }
}
extern "C" fn on_players_failed(_this: usize) {
    PLAYERS.store(0, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_players_done(_this: usize) {
    PLAYERS.store(1, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_rules_failed(_this: usize) {
    RULES.store(0, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_rules_done(_this: usize) {
    RULES.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// `--direct <steamclient.dll> --test servers --app <id>`: the server browser
/// through our DLL: an internet list, then players and rules of one server.
/// Public lobbies of the attached game, worldwide: id, owner, members, data.
fn lobbies_test(dll: &str) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    const V: &str = "SteamMatchMaking009";
    let mm = generic(client, user, pipe, c"SteamMatchMaking009".as_ptr());
    let m = |name: &str| slot(mm, steamclient::slot_of(V, name).unwrap_or_else(|| panic!("{name}")));
    let get_callback: extern "C" fn(i32, *mut CallbackMsg) -> bool = unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) };
    let free_callback: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) };
    let distance: extern "C" fn(usize, i32) = unsafe { std::mem::transmute(m("AddRequestLobbyListDistanceFilter")) };
    let request: extern "C" fn(usize) -> u64 = unsafe { std::mem::transmute(m("RequestLobbyList")) };
    distance(mm, 3);
    println!("RequestLobbyList = call {}", request(mm));
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        while get_callback(pipe, &mut msg) {
            free_callback(pipe);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let by_index: extern "C" fn(usize, i32) -> u64 = unsafe { std::mem::transmute(m("GetLobbyByIndex")) };
    let members: extern "C" fn(usize, u64) -> i32 = unsafe { std::mem::transmute(m("GetNumLobbyMembers")) };
    let limit: extern "C" fn(usize, u64) -> i32 = unsafe { std::mem::transmute(m("GetLobbyMemberLimit")) };
    let count: extern "C" fn(usize, u64) -> i32 = unsafe { std::mem::transmute(m("GetLobbyDataCount")) };
    let by_idx: extern "C" fn(usize, u64, i32, *mut c_char, i32, *mut c_char, i32) -> bool = unsafe { std::mem::transmute(m("GetLobbyDataByIndex")) };
    for i in 0..100 {
        let id = by_index(mm, i);
        if id == 0 {
            break;
        }
        let mut data = String::new();
        for k in 0..count(mm, id) {
            let (mut key, mut val) = ([0 as c_char; 256], [0 as c_char; 512]);
            if by_idx(mm, id, k, key.as_mut_ptr(), 256, val.as_mut_ptr(), 512) {
                data += &format!(" {}={:?}", s(key.as_ptr()), s(val.as_ptr()));
            }
        }
        println!("lobby {id} members {}/{}{data}", members(mm, id), limit(mm, id));
    }
    std::process::exit(0);
}

fn servers_test(dll: &str, app: u32) -> ! {
    use std::sync::atomic::Ordering::SeqCst;
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    const V: &str = "SteamMatchMakingServers002";
    let mms = generic(client, user, pipe, c"SteamMatchMakingServers002".as_ptr());
    let m = |name: &str| slot(mms, steamclient::slot_of(V, name).unwrap_or_else(|| panic!("{name}")));
    let get_callback: extern "C" fn(i32, *mut CallbackMsg) -> bool = unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) };
    let free_callback: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) };
    let pump = |until: &dyn Fn() -> bool| {
        let end = Instant::now() + Duration::from_secs(40);
        while Instant::now() < end && !until() {
            let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
            while get_callback(pipe, &mut msg) {
                free_callback(pipe);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let list_vt: [usize; 3] = [on_responded as *const () as usize, on_failed as *const () as usize, on_complete as *const () as usize];
    let list = Listener { vtable: list_vt.as_ptr() };
    let request: extern "C" fn(usize, u32, *const *const u8, u32, *const Listener) -> usize = unsafe { std::mem::transmute(m("RequestInternetServerList")) };
    let filters = [0u8; 512];
    let fp = filters.as_ptr();
    let h = request(mms, app, &fp, 0, &list);
    pump(&|| COMPLETE.load(SeqCst) >= 0);
    let count: extern "C" fn(usize, usize) -> i32 = unsafe { std::mem::transmute(m("GetServerCount")) };
    println!("list: {} servers, {} answered, {} didn't, complete {}", count(mms, h), RESPONDED.load(SeqCst), FAILED.load(SeqCst), COMPLETE.load(SeqCst));
    let details: extern "C" fn(usize, usize, i32) -> *const u8 = unsafe { std::mem::transmute(m("GetServerDetails")) };
    let mut best: Option<(u32, u16, i32)> = None;
    for i in 0..count(mms, h) {
        let d = details(mms, h, i);
        let d = unsafe { std::slice::from_raw_parts(d, 376) };
        if d[12] == 0 {
            continue;
        }
        let ping = i32::from_le_bytes(d[8..12].try_into().unwrap());
        let players = i32::from_le_bytes(d[148..152].try_into().unwrap());
        if i < 400 && best.is_none_or(|b| players > 0 && ping < b.2) {
            best = Some((u32::from_le_bytes(d[4..8].try_into().unwrap()), u16::from_le_bytes(d[2..4].try_into().unwrap()), ping));
        }
        if RESPONDED.load(SeqCst) > 0 && i < 3 {
            println!("  {:?} on {} {}/{} players, ping {ping}", s(d[172..].as_ptr().cast()), s(d[46..].as_ptr().cast()), players, i32::from_le_bytes(d[152..156].try_into().unwrap()));
        }
    }
    let mut ok = COMPLETE.load(SeqCst) == 0 && RESPONDED.load(SeqCst) > 0;
    if let Some((ip, port, ping)) = best {
        println!("server {}:{port} (ping {ping}):", std::net::Ipv4Addr::from(ip));
        let players_vt: [usize; 3] = [on_player as *const () as usize, on_players_failed as *const () as usize, on_players_done as *const () as usize];
        let rules_vt: [usize; 3] = [on_rule as *const () as usize, on_rules_failed as *const () as usize, on_rules_done as *const () as usize];
        let (p, r) = (Listener { vtable: players_vt.as_ptr() }, Listener { vtable: rules_vt.as_ptr() });
        let ask_players: extern "C" fn(usize, u32, u16, *const Listener) -> i32 = unsafe { std::mem::transmute(m("PlayerDetails")) };
        let ask_rules: extern "C" fn(usize, u32, u16, *const Listener) -> i32 = unsafe { std::mem::transmute(m("ServerRules")) };
        ask_players(mms, ip, port, &p);
        pump(&|| PLAYERS.load(SeqCst) >= 0);
        COUNT.store(0, SeqCst);
        ask_rules(mms, ip, port, &r);
        pump(&|| RULES.load(SeqCst) >= 0);
        println!("players {}, rules {}", PLAYERS.load(SeqCst), RULES.load(SeqCst));
        ok &= PLAYERS.load(SeqCst) == 1;
    }
    std::process::exit(if ok { 0 } else { 1 });
}

/// `--direct <steamclient.dll> --test input`: Steam Input bindings and pads.
fn input_test(dll: &str) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(client, 0)) };
    let pipe = pipe(client);
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(slot(client, 2)) };
    let user = connect(client, pipe);
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(slot(client, 12)) };
    const V: &str = "SteamInput006";
    let input = generic(client, user, pipe, c"SteamInput006".as_ptr());
    let m = |name: &str| slot(input, steamclient::slot_of(V, name).unwrap_or_else(|| panic!("{name}")));
    let init: extern "C" fn(usize, bool) -> bool = unsafe { std::mem::transmute(m("Init")) };
    let connected: extern "C" fn(usize, *mut u64) -> i32 = unsafe { std::mem::transmute(m("GetConnectedControllers")) };
    println!("Init = {}", init(input, false));
    std::thread::sleep(Duration::from_secs(8));
    let mut handles = [0u64; 16];
    println!("controllers: {}", connected(input, handles.as_mut_ptr()));
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    if let (Some(dll), Some("input")) = (arg("--direct"), arg("--test").as_deref()) {
        input_test(&dll);
    }
    if let (Some(dll), Some("lobbies")) = (arg("--direct"), arg("--test").as_deref()) {
        lobbies_test(&dll);
    }
    if let (Some(dll), Some("servers")) = (arg("--direct"), arg("--test").as_deref()) {
        servers_test(&dll, arg("--app").and_then(|a| a.parse().ok()).unwrap_or(440));
    }
    if let (Some(dll), Some("ugc")) = (arg("--direct"), arg("--test").as_deref()) {
        ugc_test(&dll);
    }
    if let (Some(dll), Some("inventory")) = (arg("--direct"), arg("--test").as_deref()) {
        inventory_test(&dll);
    }
    if let Some(dll) = arg("--direct") {
        direct(&dll, arg("--role").as_deref() == Some("host"), arg("--peer").and_then(|p| p.parse().ok()).unwrap_or(0));
    }
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let dll: Vec<u16> = dir.join(if cfg!(target_pointer_width = "64") { "steam_api64.dll" } else { "steam_api.dll" }).to_string_lossy().encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(dll.as_ptr()) };
    assert!(!module.is_null(), "steam_api64.dll not next to the probe");
    let api = Api(module);

    let ok = if let Some(init) = api.f("SteamAPI_InitFlat") {
        let mut err = [0 as c_char; 1024];
        let init: extern "C" fn(*mut c_char) -> i32 = unsafe { std::mem::transmute(init) };
        let r = init(err.as_mut_ptr());
        println!("SteamAPI_InitFlat = {r} {}", s(err.as_ptr()));
        r == 0
    } else {
        let r = call!(api, "SteamAPI_Init", fn() -> bool,);
        println!("SteamAPI_Init = {r}");
        r
    };
    if !ok {
        std::process::exit(2);
    }
    call!(api, "SteamAPI_ManualDispatch_Init", fn() -> (),);
    let pipe = call!(api, "SteamAPI_GetHSteamPipe", fn() -> i32,);

    let user = api.iface("SteamUser", &["v023", "v022", "v021"]);
    let friends = api.iface("SteamFriends", &["v018", "v017"]);
    let utils = api.iface("SteamUtils", &["v010", "v009"]);
    let apps = api.iface("SteamApps", &["v008"]);
    let stats = api.iface("SteamUserStats", &["v013", "v012"]);
    let rs = api.iface("SteamRemoteStorage", &["v016", "v014"]);

    println!("steam id      {}", call!(api, "SteamAPI_ISteamUser_GetSteamID", fn(usize) -> u64, user));
    println!("logged on     {}", call!(api, "SteamAPI_ISteamUser_BLoggedOn", fn(usize) -> bool, user));
    println!("persona       {}", s(call!(api, "SteamAPI_ISteamFriends_GetPersonaName", fn(usize) -> *const c_char, friends)));
    println!("friends       {}", call!(api, "SteamAPI_ISteamFriends_GetFriendCount", fn(usize, i32) -> i32, friends, 4));
    println!("app id        {}", call!(api, "SteamAPI_ISteamUtils_GetAppID", fn(usize) -> u32, utils));
    println!("country       {}", s(call!(api, "SteamAPI_ISteamUtils_GetIPCountry", fn(usize) -> *const c_char, utils)));
    println!("subscribed    {}", call!(api, "SteamAPI_ISteamApps_BIsSubscribed", fn(usize) -> bool, apps));
    println!("language      {}", s(call!(api, "SteamAPI_ISteamApps_GetCurrentGameLanguage", fn(usize) -> *const c_char, apps)));

    if api.f("SteamAPI_ISteamUserStats_RequestCurrentStats").is_some() {
        call!(api, "SteamAPI_ISteamUserStats_RequestCurrentStats", fn(usize) -> bool, stats);
    }
    let n = call!(api, "SteamAPI_ISteamUserStats_GetNumAchievements", fn(usize) -> u32, stats);
    println!("achievements  {n}");
    for i in 0..n {
        let name = call!(api, "SteamAPI_ISteamUserStats_GetAchievementName", fn(usize, u32) -> *const c_char, stats, i);
        let (mut got, mut when) = (false, 0u32);
        call!(api, "SteamAPI_ISteamUserStats_GetAchievementAndUnlockTime", fn(usize, *const c_char, *mut bool, *mut u32) -> bool, stats, name, &mut got, &mut when);
        let key = CString::new("name").unwrap();
        let display = call!(api, "SteamAPI_ISteamUserStats_GetAchievementDisplayAttribute", fn(usize, *const c_char, *const c_char) -> *const c_char, stats, name, key.as_ptr());
        println!("  {:<24} {:<28} unlocked={got} at {when}", s(name), s(display));
    }
    for stat in ["NumGames", "NumWins", "NumLosses", "FeetTraveled", "MaxFeetTraveled"] {
        let c = CString::new(stat).unwrap();
        let mut i = 0i32;
        let mut f = 0f32;
        if call!(api, "SteamAPI_ISteamUserStats_GetStatInt32", fn(usize, *const c_char, *mut i32) -> bool, stats, c.as_ptr(), &mut i) {
            println!("  stat {stat} = {i}");
        } else if call!(api, "SteamAPI_ISteamUserStats_GetStatFloat", fn(usize, *const c_char, *mut f32) -> bool, stats, c.as_ptr(), &mut f) {
            println!("  stat {stat} = {f}");
        }
    }

    let mut wrote = false;
    if let Some(name) = arg("--unlock") {
        let c = CString::new(name.clone()).unwrap();
        let r = call!(api, "SteamAPI_ISteamUserStats_SetAchievement", fn(usize, *const c_char) -> bool, stats, c.as_ptr());
        println!("SetAchievement({name}) = {r}");
        wrote = true;
    }
    if let Some(kv) = arg("--stat") {
        let (k, v) = kv.split_once('=').expect("NAME=VALUE");
        let c = CString::new(k).unwrap();
        let r = call!(api, "SteamAPI_ISteamUserStats_SetStatInt32", fn(usize, *const c_char, i32) -> bool, stats, c.as_ptr(), v.parse().unwrap());
        println!("SetStat({k}={v}) = {r}");
        wrote = true;
    }
    if wrote {
        println!("StoreStats = {}", call!(api, "SteamAPI_ISteamUserStats_StoreStats", fn(usize) -> bool, stats));
    }
    if args.iter().any(|a| a == "--cloud") {
        let (name, data) = (CString::new("probe/test.txt").unwrap(), b"hello from the probe");
        let w = call!(api, "SteamAPI_ISteamRemoteStorage_FileWrite", fn(usize, *const c_char, *const u8, i32) -> bool, rs, name.as_ptr(), data.as_ptr(), data.len() as i32);
        let mut buf = [0u8; 64];
        let r = call!(api, "SteamAPI_ISteamRemoteStorage_FileRead", fn(usize, *const c_char, *mut u8, i32) -> i32, rs, name.as_ptr(), buf.as_mut_ptr(), 64);
        let count = call!(api, "SteamAPI_ISteamRemoteStorage_GetFileCount", fn(usize) -> i32, rs);
        println!("cloud write={w} read={r} ({:?}) files={count}", std::str::from_utf8(&buf[..r.max(0) as usize]).unwrap_or("?"));
        call!(api, "SteamAPI_ISteamRemoteStorage_FileDelete", fn(usize, *const c_char) -> bool, rs, name.as_ptr());
    }
    let players = call!(api, "SteamAPI_ISteamUserStats_GetNumberOfCurrentPlayers", fn(usize) -> u64, stats);
    let tickets = args.iter().any(|a| a == "--ticket");
    let mut my_ticket = Vec::new();
    if tickets {
        let mut buf = vec![0u8; 1024];
        let mut len = 0u32;
        let h = if api.f("SteamAPI_ISteamUser_GetAuthTicketForWebApi").is_some() {
            call!(api, "SteamAPI_ISteamUser_GetAuthSessionTicket", fn(usize, *mut u8, i32, *mut u32, *const c_void) -> u32, user, buf.as_mut_ptr(), 1024, &mut len, std::ptr::null())
        } else {
            call!(api, "SteamAPI_ISteamUser_GetAuthSessionTicket", fn(usize, *mut u8, i32, *mut u32) -> u32, user, buf.as_mut_ptr(), 1024, &mut len)
        };
        my_ticket = buf[..len as usize].to_vec();
        println!("GetAuthSessionTicket = handle {h}, {len} bytes");
        let r = call!(api, "SteamAPI_ISteamUser_RequestEncryptedAppTicket", fn(usize, *const u8, i32) -> u64, user, b"probe".as_ptr(), 5);
        println!("RequestEncryptedAppTicket = call {r}");
    }
    let mut begun = false;
    let lobby_test = args.iter().any(|a| a == "--lobby");
    let extras = args.iter().any(|a| a == "--extras");
    let http = api.iface("SteamHTTP", &["v003"]);
    if extras {
        let n = CString::new("Feet Traveled").unwrap();
        println!("FindLeaderboard = call {}", call!(api, "SteamAPI_ISteamUserStats_FindLeaderboard", fn(usize, *const c_char) -> u64, stats, n.as_ptr()));
        println!("RequestGlobalAchievementPercentages = call {}", call!(api, "SteamAPI_ISteamUserStats_RequestGlobalAchievementPercentages", fn(usize) -> u64, stats));
        let me = call!(api, "SteamAPI_ISteamUser_GetSteamID", fn(usize) -> u64, user);
        println!("GetLargeFriendAvatar = {}", call!(api, "SteamAPI_ISteamFriends_GetLargeFriendAvatar", fn(usize, u64) -> i32, friends, me));
        let url = CString::new("https://api.steampowered.com/ISteamWebAPIUtil/GetServerInfo/v1/").unwrap();
        let h = call!(api, "SteamAPI_ISteamHTTP_CreateHTTPRequest", fn(usize, i32, *const c_char) -> u32, http, 1, url.as_ptr());
        let mut c = 0u64;
        let sent = call!(api, "SteamAPI_ISteamHTTP_SendHTTPRequest", fn(usize, u32, *mut u64) -> bool, http, h, &mut c);
        println!("HTTP request {h} sent={sent} call {c}");
    }
    if args.iter().any(|a| a == "--sockets") {
        let me = call!(api, "SteamAPI_ISteamUser_GetSteamID", fn(usize) -> u64, user);
        sockets_test(&api, pipe, me);
    }
    let mm = api.iface("SteamMatchmaking", &["v009"]);
    let mut my_lobby = 0u64;
    let mut searched_at: Option<Instant> = None;
    if lobby_test {
        let call = call!(api, "SteamAPI_ISteamMatchmaking_CreateLobby", fn(usize, i32, i32) -> u64, mm, 2, 4);
        println!("CreateLobby = call {call}");
    }

    // Pump callbacks for a few seconds, like a game's frame loop.
    let until = Instant::now() + Duration::from_secs(if wrote || tickets || lobby_test || extras { 25 } else { 5 });
    while Instant::now() < until {
        call!(api, "SteamAPI_ManualDispatch_RunFrame", fn(i32) -> (), pipe);
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        while call!(api, "SteamAPI_ManualDispatch_GetNextCallback", fn(i32, *mut CallbackMsg) -> bool, pipe, &mut msg) {
            let mut line = format!("callback {} ({} bytes)", msg.callback, msg.size);
            if msg.callback == 703 {
                let call_id = unsafe { (msg.param as *const u64).read_unaligned() };
                let expected = unsafe { (msg.param.add(8) as *const i32).read_unaligned() };
                let mut buf = [0u8; 512];
                let mut failed = false;
                let ok = call!(api, "SteamAPI_ManualDispatch_GetAPICallResult", fn(i32, u64, *mut u8, i32, i32, *mut bool) -> bool, pipe, call_id, buf.as_mut_ptr(), 512, expected, &mut failed);
                line += &format!(" -> call {call_id} result {expected} ok={ok} failed={failed}");
                if expected == 154 {
                    let mut n = 0u32;
                    let mut tb = vec![0u8; 2048];
                    let got = call!(api, "SteamAPI_ISteamUser_GetEncryptedAppTicket", fn(usize, *mut u8, i32, *mut u32) -> bool, user, tb.as_mut_ptr(), 2048, &mut n);
                    line += &format!(" EncryptedAppTicket result={} got={got} {n} bytes", i32::from_le_bytes(buf[0..4].try_into().unwrap()));
                }
                if expected == 1104 {
                    let lb = u64::from_le_bytes(buf[0..8].try_into().unwrap());
                    let name = call!(api, "SteamAPI_ISteamUserStats_GetLeaderboardName", fn(usize, u64) -> *const c_char, stats, lb);
                    let count = call!(api, "SteamAPI_ISteamUserStats_GetLeaderboardEntryCount", fn(usize, u64) -> i32, stats, lb);
                    line += &format!(" LeaderboardFindResult found={} {lb} {:?} entries={count}", buf[8], s(name));
                    if buf[8] == 1 {
                        call!(api, "SteamAPI_ISteamUserStats_DownloadLeaderboardEntries", fn(usize, u64, i32, i32, i32) -> u64, stats, lb, 0, 1, 5);
                    }
                }
                if expected == 1105 {
                    let set = u64::from_le_bytes(buf[8..16].try_into().unwrap());
                    let n = i32::from_le_bytes(buf[16..20].try_into().unwrap());
                    line += &format!(" LeaderboardScoresDownloaded {n}:");
                    for i in 0..n {
                        let mut e = [0u8; 32];
                        call!(api, "SteamAPI_ISteamUserStats_GetDownloadedLeaderboardEntry", fn(usize, u64, i32, *mut u8, *mut i32, i32) -> bool, stats, set, i, e.as_mut_ptr(), std::ptr::null_mut(), 0);
                        line += &format!(" #{} {}={}", i32::from_le_bytes(e[8..12].try_into().unwrap()), u64::from_le_bytes(e[0..8].try_into().unwrap()), i32::from_le_bytes(e[12..16].try_into().unwrap()));
                    }
                }
                if expected == 1110 {
                    let mut name = [0 as c_char; 128];
                    let (mut pct, mut got) = (0f32, false);
                    let i = call!(api, "SteamAPI_ISteamUserStats_GetMostAchievedAchievementInfo", fn(usize, *mut c_char, u32, *mut f32, *mut bool) -> i32, stats, name.as_mut_ptr(), 128, &mut pct, &mut got);
                    line += &format!(" GlobalAchievementPercentagesReady result={}; most achieved #{i} {} {pct}% mine={got}", i32::from_le_bytes(buf[8..12].try_into().unwrap()), s(name.as_ptr()));
                }
                if expected == 2101 {
                    let status = i32::from_le_bytes(buf[20..24].try_into().unwrap());
                    let size = u32::from_le_bytes(buf[24..28].try_into().unwrap());
                    let mut body = vec![0u8; size as usize];
                    let ok = call!(api, "SteamAPI_ISteamHTTP_GetHTTPResponseBodyData", fn(usize, u32, *mut u8, u32) -> bool, http, u32::from_le_bytes(buf[0..4].try_into().unwrap()), body.as_mut_ptr(), size);
                    line += &format!(" HTTPRequestCompleted status={status} body={size} bytes ok={ok}: {}", String::from_utf8_lossy(&body[..body.len().min(60)]).replace(['\n', '\r'], " "));
                }
                if expected == 513 {
                    let r = i32::from_le_bytes(buf[0..4].try_into().unwrap());
                    my_lobby = u64::from_le_bytes(buf[8..16].try_into().unwrap());
                    let (k, v) = (CString::new("sir_probe").unwrap(), CString::new("hello").unwrap());
                    let set = call!(api, "SteamAPI_ISteamMatchmaking_SetLobbyData", fn(usize, u64, *const c_char, *const c_char) -> bool, mm, my_lobby, k.as_ptr(), v.as_ptr());
                    line += &format!(" LobbyCreated result={r} lobby={my_lobby}; SetLobbyData={set}");
                    searched_at = Some(Instant::now());
                }
                if expected == 510 {
                    let n = u32::from_le_bytes(buf[0..4].try_into().unwrap());
                    line += &format!(" LobbyMatchList {n} lobbies:");
                    for i in 0..n {
                        let id = call!(api, "SteamAPI_ISteamMatchmaking_GetLobbyByIndex", fn(usize, i32) -> u64, mm, i as i32);
                        let k = CString::new("sir_probe").unwrap();
                        let v = call!(api, "SteamAPI_ISteamMatchmaking_GetLobbyData", fn(usize, u64, *const c_char) -> *const c_char, mm, id, k.as_ptr());
                        line += &format!(" {id}{}(sir_probe={:?})", if id == my_lobby { "*" } else { "" }, s(v));
                    }
                    call!(api, "SteamAPI_ISteamMatchmaking_LeaveLobby", fn(usize, u64) -> (), mm, my_lobby);
                    line += "; left";
                }
                if call_id == players && expected == 1107 {
                    line += &format!(" players={}", i32::from_le_bytes(buf[4..8].try_into().unwrap()));
                }
            } else if msg.callback == 1102 || msg.callback == 1101 {
                let r = unsafe { (msg.param.add(8) as *const i32).read_unaligned() };
                line += &format!(" result={r}");
            } else if msg.callback == 504 || msg.callback == 505 || msg.callback == 506 {
                let lobby = unsafe { (msg.param as *const u64).read_unaligned() };
                line += &format!(" lobby {lobby}");
                if msg.callback == 505 && lobby_test {
                    let k = CString::new("sir_probe").unwrap();
                    let v = call!(api, "SteamAPI_ISteamMatchmaking_GetLobbyData", fn(usize, u64, *const c_char) -> *const c_char, mm, lobby, k.as_ptr());
                    let n = call!(api, "SteamAPI_ISteamMatchmaking_GetNumLobbyMembers", fn(usize, u64) -> i32, mm, lobby);
                    line += &format!(" data sir_probe={:?} members={n}", s(v));
                }
            } else if msg.callback == 334 {
                let img = i32::from_le_bytes(unsafe { std::slice::from_raw_parts(msg.param.add(8), 4) }.try_into().unwrap());
                let (mut w, mut h) = (0u32, 0u32);
                let ok = call!(api, "SteamAPI_ISteamUtils_GetImageSize", fn(usize, i32, *mut u32, *mut u32) -> bool, utils, img, &mut w, &mut h);
                let mut rgba = vec![0u8; (w * h * 4) as usize];
                let got = call!(api, "SteamAPI_ISteamUtils_GetImageRGBA", fn(usize, i32, *mut u8, i32) -> bool, utils, img, rgba.as_mut_ptr(), rgba.len() as i32);
                line += &format!(" AvatarImageLoaded image {img}: {w}x{h} size_ok={ok} rgba_ok={got}");
            } else if msg.callback == 163 {
                let r = unsafe { (msg.param.add(4) as *const i32).read_unaligned() };
                line += &format!(" GetAuthSessionTicketResponse result={r}");
                if r == 1 && !begun {
                    begun = true;
                    let me = call!(api, "SteamAPI_ISteamUser_GetSteamID", fn(usize) -> u64, user);
                    let b = call!(api, "SteamAPI_ISteamUser_BeginAuthSession", fn(usize, *const u8, i32, u64) -> i32, user, my_ticket.as_ptr(), my_ticket.len() as i32, me);
                    line += &format!("; BeginAuthSession(own ticket) = {b}");
                }
            } else if msg.callback == 143 {
                let resp = unsafe { (msg.param.add(8) as *const i32).read_unaligned() };
                let owner = unsafe { (msg.param.add(16) as *const u64).read_unaligned() };
                line += &format!(" ValidateAuthTicketResponse response={resp} owner={owner}");
            } else if msg.callback == 1103 {
                let name = unsafe { CStr::from_ptr(msg.param.add(9) as *const c_char) };
                line += &format!(" achievement={}", name.to_string_lossy());
            }
            println!("{line}");
            call!(api, "SteamAPI_ManualDispatch_FreeLastCallback", fn(i32) -> (), pipe);
        }
        if searched_at.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
            searched_at = None;
            let (k, v) = (CString::new("sir_probe").unwrap(), CString::new("hello").unwrap());
            call!(api, "SteamAPI_ISteamMatchmaking_AddRequestLobbyListStringFilter", fn(usize, *const c_char, *const c_char, i32) -> (), mm, k.as_ptr(), v.as_ptr(), 0);
            call!(api, "SteamAPI_ISteamMatchmaking_AddRequestLobbyListDistanceFilter", fn(usize, i32) -> (), mm, 3);
            let c = call!(api, "SteamAPI_ISteamMatchmaking_RequestLobbyList", fn(usize) -> u64, mm);
            println!("RequestLobbyList = call {c}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    call!(api, "SteamAPI_Shutdown", fn() -> (),);
    println!("done");
}

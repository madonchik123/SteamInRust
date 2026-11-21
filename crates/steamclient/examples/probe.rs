//! A tiny "game" for testing the steamclient replacement: loads a real
//! `steam_api64.dll` (put it next to this exe), initialises it like a game
//! does and prints what Steam answers.
//!
//! `probe [--unlock ACH_NAME] [--stat NAME=VALUE] [--cloud]`
//! Writes only when asked (use with Spacewar, app 480).

use std::ffi::{CStr, CString, c_char, c_void};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

// The same two calls on macOS and Linux, through dlopen.
#[cfg(unix)]
unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[cfg(unix)]
#[allow(non_snake_case)]
unsafe fn LoadLibraryW(name: *const u16) -> *mut c_void {
    let len = (0..).take_while(|&i| unsafe { *name.add(i) } != 0).count();
    let path = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, len) });
    let c = CString::new(path).unwrap_or_default();
    unsafe { dlopen(c.as_ptr(), 2) }
}

#[cfg(unix)]
#[allow(non_snake_case)]
unsafe fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void {
    unsafe { dlsym(module, name.cast()) }
}

/// The game-side Steam API library this probe loads, next to the probe.
const STEAM_API: &str = if cfg!(target_os = "macos") {
    "libsteam_api.dylib"
} else if cfg!(unix) {
    "libsteam_api.so"
} else if cfg!(target_pointer_width = "64") {
    "steam_api64.dll"
} else {
    "steam_api.dll"
};

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

/// `--direct <steamclient.dll> --test cert`: our DLL asks Steam (through a
/// signed-in SteamInRust) for a networking certificate; see the DLL log.
fn cert_test(dll: &str) -> ! {
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
    const V: &str = "SteamNetworkingSockets012";
    let sockets = generic(client, user, pipe, c"SteamNetworkingSockets012".as_ptr());
    assert!(sockets != 0);
    let init: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(slot(sockets, steamclient::slot_of(V, "InitAuthentication").unwrap())) };
    println!("InitAuthentication = {}", init(sockets));
    std::thread::sleep(Duration::from_secs(10));
    std::process::exit(0);
}

/// One interface method by name, as a plain function.
fn method(obj: usize, version: &str, name: &str) -> usize {
    slot(obj, steamclient::slot_of(version, name).unwrap_or_else(|| panic!("{version}::{name}")))
}

/// Copies callbacks off a pipe until the wanted one arrives.
struct Pump {
    get: extern "C" fn(i32, *mut CallbackMsg) -> bool,
    free: extern "C" fn(i32) -> bool,
}

impl Pump {
    fn next(&self, pipe: i32) -> Option<(i32, Vec<u8>)> {
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        if !(self.get)(pipe, &mut msg) {
            return None;
        }
        let data = unsafe { std::slice::from_raw_parts(msg.param, msg.size.max(0) as usize) }.to_vec();
        (self.free)(pipe);
        Some((msg.callback, data))
    }

    fn wait(&self, pipe: i32, wanted: &[i32], secs: u64) -> Option<(i32, Vec<u8>)> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            while let Some((id, data)) = self.next(pipe) {
                if wanted.contains(&id) {
                    return Some((id, data));
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

/// One A2S exchange with a local server: the reply after its challenge.
fn a2s(sock: &std::net::UdpSocket, to: &str, query: &[u8]) -> Option<Vec<u8>> {
    let mut buf = [0u8; 1500];
    sock.send_to(query, to).ok()?;
    let (n, _) = sock.recv_from(&mut buf).ok()?;
    if n < 9 || buf[4] != b'A' {
        return Some(buf[..n].to_vec());
    }
    let mut q = query.to_vec();
    if query[4] != b'T' {
        q.truncate(5);
    }
    q.extend_from_slice(&buf[5..9]);
    sock.send_to(&q, to).ok()?;
    let (n, _) = sock.recv_from(&mut buf).ok()?;
    Some(buf[..n].to_vec())
}

fn check(results: &mut Vec<(&'static str, bool)>, what: &'static str, ok: bool, detail: String) {
    println!("{} {what}: {detail}", if ok { "PASS" } else { "FAIL" });
    results.push((what, ok));
}

/// `--direct <steamclient.dll> --test gameserver [--port N] [--bad-token]`:
/// hosts a Spacewar (480) server through our DLL for about a minute and
/// plays a client against it, through a signed-in SteamInRust. Reads the
/// player's stats but never writes them. `--player-net` also connects the
/// player to the server with SteamNetworkingSockets. The server's answer never
/// reached a test session that wasn't reported as playing the game (maybe why),
/// so try it from a real launch; this tool doesn't report playing.
fn gameserver_test(dll: &str, port: u16, bad_token: bool, test_player_net: bool) -> ! {
    use std::sync::atomic::Ordering::SeqCst;
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    const C: &str = "SteamClient021";
    const G: &str = "SteamGameServer015";
    const ST: &str = "SteamGameServerStats001";
    const U: &str = "SteamUser023";
    const NS: &str = "SteamNetworkingSockets012";
    const NM: &str = "SteamNetworkingMessages002";
    const MS: &str = "SteamMatchMakingServers002";
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let new_pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(method(client, C, "CreateSteamPipe")) };
    let connect: extern "C" fn(usize, i32) -> i32 = unsafe { std::mem::transmute(method(client, C, "ConnectToGlobalUser")) };
    let local: extern "C" fn(usize, *mut i32, i32) -> i32 = unsafe { std::mem::transmute(method(client, C, "CreateLocalUser")) };
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(method(client, C, "GetISteamGenericInterface")) };
    let player_pipe = new_pipe(client);
    let player = connect(client, player_pipe);
    let mut server_pipe = new_pipe(client);
    let server_user = local(client, &mut server_pipe, 3);
    let pump = Pump {
        get: unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) },
        free: unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) },
    };
    let gs = generic(client, server_user, server_pipe, c"SteamGameServer015".as_ptr());
    assert!(gs != 0, "no game server interface");
    let mut results = Vec::new();

    macro_rules! m {
        ($obj:expr, $v:expr, $name:literal, fn($($t:ty),*) -> $r:ty, $($a:expr),*) => {{
            let f: extern "C" fn(usize, $($t),*) -> $r = unsafe { std::mem::transmute(method($obj, $v, $name)) };
            f($obj, $($a),*)
        }};
    }
    let query_port = port + 1;
    m!(gs, G, "InitGameServer", fn(u32, u16, u16, u32, u32, *const c_char) -> bool, 0, port, query_port, 0, 480, c"1.0.0.0".as_ptr());
    m!(gs, G, "SetProduct", fn(*const c_char) -> (), c"SteamworksExample".as_ptr());
    m!(gs, G, "SetGameDescription", fn(*const c_char) -> (), c"SteamInRust test".as_ptr());
    m!(gs, G, "SetModDir", fn(*const c_char) -> (), c"spacewar".as_ptr());
    m!(gs, G, "SetDedicatedServer", fn(bool) -> (), true);

    if bad_token {
        m!(gs, G, "LogOn", fn(*const c_char) -> (), c"not-a-real-login-token".as_ptr());
        let r = pump.wait(server_pipe, &[101, 102], 30);
        let code = r.as_ref().filter(|(id, _)| *id == 102).map(|(_, d)| i32::from_le_bytes(d[..4].try_into().unwrap()));
        check(&mut results, "Steam refuses a bad login token", code.is_some(), format!("{r:?}"));
        std::process::exit(if code.is_some() { 0 } else { 1 });
    }

    m!(gs, G, "LogOnAnonymous", fn() -> (),);
    m!(gs, G, "SetMaxPlayerCount", fn(i32) -> (), 4);
    m!(gs, G, "SetBotPlayerCount", fn(i32) -> (), 1);
    m!(gs, G, "SetServerName", fn(*const c_char) -> (), c"SteamInRust test server".as_ptr());
    m!(gs, G, "SetMapName", fn(*const c_char) -> (), c"sirtest".as_ptr());
    m!(gs, G, "SetGameTags", fn(*const c_char) -> (), c"test".as_ptr());
    m!(gs, G, "SetKeyValue", fn(*const c_char, *const c_char) -> (), c"mode".as_ptr(), c"ffa".as_ptr());
    m!(gs, G, "SetAdvertiseServerActive", fn(bool) -> (), true);

    // 1. Signed in.
    let up = pump.wait(server_pipe, &[101, 102], 30);
    let signed = matches!(up, Some((101, _))) && m!(gs, G, "BLoggedOn", fn() -> bool,);
    let mut id = 0u64;
    m!(gs, G, "GetSteamID", fn(*mut u64) -> *mut u64, &mut id);
    let mut ip_struct = [0u8; 20];
    m!(gs, G, "GetPublicIP", fn(*mut [u8; 20]) -> *mut [u8; 20], &mut ip_struct);
    let ip = std::net::Ipv4Addr::from(u32::from_le_bytes(ip_struct[..4].try_into().unwrap()));
    check(&mut results, "anonymous sign-in", signed && id >> 52 == 0x014, format!("Steam ID {id}"));
    if !signed {
        std::process::exit(1);
    }
    println!("   BSecure = {}", m!(gs, G, "BSecure", fn() -> bool,));

    // 2. Answers server-browser queries on its query port.
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let to = format!("127.0.0.1:{query_port}");
    std::thread::sleep(Duration::from_millis(1200));
    let info = a2s(&sock, &to, b"\xFF\xFF\xFF\xFFTSource Engine Query\0").unwrap_or_default();
    let text = String::from_utf8_lossy(&info).into_owned();
    check(&mut results, "A2S_INFO", info.get(4) == Some(&b'I') && text.contains("SteamInRust test server") && text.contains("sirtest"), format!("{} bytes", info.len()));
    let rules = a2s(&sock, &to, &[0xFF, 0xFF, 0xFF, 0xFF, b'V', 0xFF, 0xFF, 0xFF, 0xFF]).unwrap_or_default();
    check(&mut results, "A2S_RULES", rules.get(4) == Some(&b'E') && String::from_utf8_lossy(&rules).contains("ffa"), format!("{} bytes", rules.len()));

    // 3. Listed by Steam's master server.
    std::thread::sleep(Duration::from_secs(5));
    let mms = generic(client, player, player_pipe, c"SteamMatchMakingServers002".as_ptr());
    let list_vt: [usize; 3] = [on_responded as *const () as usize, on_failed as *const () as usize, on_complete as *const () as usize];
    let listener = Listener { vtable: list_vt.as_ptr() };
    let mut filter = [0u8; 512];
    filter[..8].copy_from_slice(b"gameaddr");
    let dotted = ip.to_string();
    filter[256..256 + dotted.len()].copy_from_slice(dotted.as_bytes());
    let listed = |pump: &Pump| -> bool {
        COMPLETE.store(-1, SeqCst);
        let fp = filter.as_ptr();
        let h = m!(mms, MS, "RequestInternetServerList", fn(u32, *const *const u8, u32, *const Listener) -> usize, 480, &fp, 1, &listener);
        let end = Instant::now() + Duration::from_secs(25);
        while Instant::now() < end && COMPLETE.load(SeqCst) < 0 {
            while pump.next(player_pipe).is_some() {}
            std::thread::sleep(Duration::from_millis(20));
        }
        let count = m!(mms, MS, "GetServerCount", fn(usize) -> i32, h);
        (0..count).any(|i| {
            let d = m!(mms, MS, "GetServerDetails", fn(usize, i32) -> *const u8, h, i);
            let d = unsafe { std::slice::from_raw_parts(d, 376) };
            u64::from_le_bytes(d[368..376].try_into().unwrap()) == id
        })
    };
    check(&mut results, "listed by Steam's server browser", listed(&pump), format!("searched {dotted} through ISteamMatchmakingServers"));

    // 4. A player's ticket, checked by Steam for the server.
    let user = generic(client, player, player_pipe, c"SteamUser023".as_ptr());
    let mut me = 0u64;
    m!(user, U, "GetSteamID", fn(*mut u64) -> *mut u64, &mut me);
    let mut ticket = vec![0u8; 2560];
    let mut len = 0u32;
    let handle = m!(user, U, "GetAuthSessionTicket", fn(*mut u8, i32, *mut u32, *const u8) -> u32, ticket.as_mut_ptr(), 2560, &mut len, std::ptr::null());
    let accepted = pump.wait(player_pipe, &[163], 20).is_some();
    let begin = m!(gs, G, "BeginAuthSession", fn(*const u8, i32, u64) -> i32, ticket.as_ptr(), len as i32, me);
    let verdict = pump.wait(server_pipe, &[143], 25);
    let response = verdict.as_ref().map(|(_, d)| i32::from_le_bytes(d[8..12].try_into().unwrap()));
    check(&mut results, "player ticket accepted by Steam", accepted && begin == 0 && response == Some(0), format!("begin {begin}, response {response:?}, {len} bytes"));
    let duplicate = m!(gs, G, "BeginAuthSession", fn(*const u8, i32, u64) -> i32, ticket.as_ptr(), len as i32, me);
    check(&mut results, "second session refused", duplicate == 2, format!("{duplicate}"));
    let other = m!(gs, G, "BeginAuthSession", fn(*const u8, i32, u64) -> i32, ticket.as_ptr(), len as i32, me ^ 1);
    check(&mut results, "ticket for another player refused", other == 1, format!("{other}"));
    let license = m!(gs, G, "UserHasLicenseForApp", fn(u64, u32) -> i32, me, 480);
    let no_license = m!(gs, G, "UserHasLicenseForApp", fn(u64, u32) -> i32, me, 730);
    check(&mut results, "ownership from the ticket", license == 0 && no_license == 1, format!("480: {license}, 730: {no_license}"));

    // 5. The player's stats, read as the server (never written).
    let stats = generic(client, server_user, server_pipe, c"SteamGameServerStats001".as_ptr());
    let call = m!(stats, ST, "RequestUserStats", fn(u64) -> u64, me);
    let done = pump.wait(server_pipe, &[703], 20);
    let same_call = done.as_ref().is_some_and(|(_, d)| u64::from_le_bytes(d[..8].try_into().unwrap()) == call);
    let mut result = [0u8; 16];
    let mut failed = true;
    let got: extern "C" fn(i32, u64, *mut u8, i32, i32, *mut bool) -> bool = unsafe { std::mem::transmute(api.f("Steam_GetAPICallResult").unwrap()) };
    let have = got(server_pipe, call, result.as_mut_ptr(), 16, 1800, &mut failed);
    let eresult = i32::from_le_bytes(result[..4].try_into().unwrap());
    let mut games = -1i32;
    let read = m!(stats, ST, "GetUserStatInt32", fn(u64, *const c_char, *mut i32) -> bool, me, c"NumGames".as_ptr(), &mut games);
    check(&mut results, "player stats read as the server", same_call && have && eresult == 1 && read, format!("result {eresult}, NumGames {games}"));

    // 6. Networking from the server's identity: connection and messages.
    let server_net = generic(client, server_user, server_pipe, c"SteamNetworkingSockets012".as_ptr());
    let player_net = generic(client, player, player_pipe, c"SteamNetworkingSockets012".as_ptr());
    let mut who = [0u8; 136];
    let known = m!(server_net, NS, "GetIdentity", fn(*mut u8) -> bool, who.as_mut_ptr());
    let identity_id = u64::from_le_bytes(who[8..16].try_into().unwrap());
    check(&mut results, "networking identity is the server's", known && identity_id == id, format!("{identity_id}"));
    if test_player_net {
        let server_id = identity(id);
        let listen = m!(server_net, NS, "CreateListenSocketP2P", fn(i32, i32, *const u8) -> u32, 5, 0, std::ptr::null());
        let outgoing = m!(player_net, NS, "ConnectP2P", fn(*const u8, i32, i32, *const u8) -> u32, server_id.as_ptr(), 5, 0, std::ptr::null());
        let (mut incoming, mut server_up, mut player_up) = (0u32, false, false);
        let end = Instant::now() + Duration::from_secs(25);
        while Instant::now() < end && !(server_up && player_up) {
            while let Some((cb, b)) = pump.next(server_pipe) {
                if cb == 1221 {
                    let h = u32::from_le_bytes(b[0..4].try_into().unwrap());
                    let state = i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap());
                    if state == 1 && incoming == 0 {
                        incoming = h;
                        m!(server_net, NS, "AcceptConnection", fn(u32) -> i32, h);
                    }
                    server_up |= state == 3;
                }
            }
            while let Some((cb, b)) = pump.next(player_pipe) {
                if cb == 1221 && u32::from_le_bytes(b[0..4].try_into().unwrap()) == outgoing {
                    player_up |= i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap()) == 3;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        check(&mut results, "player connected to the server's socket", listen != 0 && server_up && player_up, format!("server {server_up}, player {player_up}"));
        let receive = |net: usize, h: u32, want: &[u8]| -> bool {
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                let mut out = [std::ptr::null_mut::<u8>(); 4];
                let n = m!(net, NS, "ReceiveMessagesOnConnection", fn(u32, *mut *mut u8, i32) -> i32, h, out.as_mut_ptr(), 4);
                for p in &out[..n.max(0) as usize] {
                    if take_message(*p).2 == want {
                        return true;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        };
        if server_up && player_up {
            m!(player_net, NS, "SendMessageToConnection", fn(u32, *const u8, u32, i32, *mut i64) -> i32, outgoing, b"to server".as_ptr(), 9, 8, std::ptr::null_mut());
            let at_server = receive(server_net, incoming, b"to server");
            m!(server_net, NS, "SendMessageToConnection", fn(u32, *const u8, u32, i32, *mut i64) -> i32, incoming, b"to player".as_ptr(), 9, 8, std::ptr::null_mut());
            let at_player = receive(player_net, outgoing, b"to player");
            check(&mut results, "data over the connection, both ways", at_server && at_player, format!("server got it: {at_server}, player got it: {at_player}"));
        }
        m!(player_net, NS, "CloseConnection", fn(u32, i32, *const c_char, bool) -> bool, outgoing, 0, std::ptr::null(), false);
        m!(server_net, NS, "CloseListenSocket", fn(u32) -> bool, listen);

        let server_msgs = generic(client, server_user, server_pipe, c"SteamNetworkingMessages002".as_ptr());
        let player_msgs = generic(client, player, player_pipe, c"SteamNetworkingMessages002".as_ptr());
        let player_id = identity(me);
        m!(player_msgs, NM, "SendMessageToUser", fn(*const u8, *const u8, u32, i32, i32) -> i32, server_id.as_ptr(), b"msg up".as_ptr(), 6, 8, 2);
        let (mut got_up, mut got_down, mut replied) = (false, false, false);
        let end = Instant::now() + Duration::from_secs(25);
        while Instant::now() < end && !(got_up && got_down) {
            while let Some((cb, _)) = pump.next(server_pipe) {
                if cb == 1251 {
                    m!(server_msgs, NM, "AcceptSessionWithUser", fn(*const u8) -> bool, player_id.as_ptr());
                }
            }
            while pump.next(player_pipe).is_some() {}
            let mut out = [std::ptr::null_mut::<u8>(); 4];
            let n = m!(server_msgs, NM, "ReceiveMessagesOnChannel", fn(i32, *mut *mut u8, i32) -> i32, 2, out.as_mut_ptr(), 4);
            for p in &out[..n.max(0) as usize] {
                let (_, from, data) = take_message(*p);
                got_up |= from == me && data == b"msg up";
            }
            if got_up && !replied {
                replied = m!(server_msgs, NM, "SendMessageToUser", fn(*const u8, *const u8, u32, i32, i32) -> i32, player_id.as_ptr(), b"msg down".as_ptr(), 8, 8, 2) == 1;
            }
            let n = m!(player_msgs, NM, "ReceiveMessagesOnChannel", fn(i32, *mut *mut u8, i32) -> i32, 2, out.as_mut_ptr(), 4);
            for p in &out[..n.max(0) as usize] {
                let (_, from, data) = take_message(*p);
                got_down |= from == id && data == b"msg down";
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        check(&mut results, "ISteamNetworkingMessages, both ways", got_up && got_down, format!("server got it: {got_up}, player got it: {got_down}"));
    } else {
        println!("NOT TESTED player-to-server networking (see --player-net; server-to-server is --test gs-net)");
    }

    // 7. Goes away cleanly.
    m!(gs, G, "EndAuthSession", fn(u64) -> (), me);
    m!(user, U, "CancelAuthTicket", fn(u32) -> (), handle);
    let after = m!(gs, G, "UserHasLicenseForApp", fn(u64, u32) -> i32, me, 480);
    check(&mut results, "session ended", after == 2, format!("{after}"));
    m!(gs, G, "SetAdvertiseServerActive", fn(bool) -> (), false);
    std::thread::sleep(Duration::from_millis(1500));
    m!(gs, G, "LogOff", fn() -> (),);
    let down = pump.wait(server_pipe, &[103], 5).is_some();
    check(&mut results, "signed off", down && !m!(gs, G, "BLoggedOn", fn() -> bool,), String::new());
    std::thread::sleep(Duration::from_secs(4));
    check(&mut results, "gone from the server browser", !listed(&pump), "searched again after signing off".into());

    let failed: Vec<&str> = results.iter().filter(|r| !r.1).map(|r| r.0).collect();
    println!("{} of {} checks passed{}", results.len() - failed.len(), results.len(), if failed.is_empty() { String::new() } else { format!("; failed: {failed:?}") });
    std::process::exit(if failed.is_empty() { 0 } else { 1 });
}

/// `--direct <steamclient.dll> --test gs-net --role host|join --id-file <path>`:
/// two game servers (one process each, neither listed) connect to each other
/// by Steam ID with SteamNetworkingSockets, then exchange messages with
/// SteamNetworkingMessages. No player identity is involved.
fn gs_net_test(dll: &str, host: bool, id_file: &str) -> ! {
    let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {dll}");
    let api = Api(module);
    let create: extern "C" fn(*const c_char, *mut i32) -> usize = unsafe { std::mem::transmute(api.f("CreateInterface").unwrap()) };
    const C: &str = "SteamClient021";
    const G: &str = "SteamGameServer015";
    const NS: &str = "SteamNetworkingSockets012";
    const NM: &str = "SteamNetworkingMessages002";
    let client = create(c"SteamClient021".as_ptr(), std::ptr::null_mut());
    let new_pipe: extern "C" fn(usize) -> i32 = unsafe { std::mem::transmute(method(client, C, "CreateSteamPipe")) };
    let local: extern "C" fn(usize, *mut i32, i32) -> i32 = unsafe { std::mem::transmute(method(client, C, "CreateLocalUser")) };
    let generic: extern "C" fn(usize, i32, i32, *const c_char) -> usize = unsafe { std::mem::transmute(method(client, C, "GetISteamGenericInterface")) };
    let mut pipe = new_pipe(client);
    let user = local(client, &mut pipe, 3);
    let pump = Pump { get: unsafe { std::mem::transmute(api.f("Steam_BGetCallback").unwrap()) }, free: unsafe { std::mem::transmute(api.f("Steam_FreeLastCallback").unwrap()) } };
    let gs = generic(client, user, pipe, c"SteamGameServer015".as_ptr());
    macro_rules! m {
        ($obj:expr, $v:expr, $name:literal, fn($($t:ty),*) -> $r:ty, $($a:expr),*) => {{
            let f: extern "C" fn(usize, $($t),*) -> $r = unsafe { std::mem::transmute(method($obj, $v, $name)) };
            f($obj, $($a),*)
        }};
    }
    let port: u16 = if host { 27925 } else { 27935 };
    m!(gs, G, "InitGameServer", fn(u32, u16, u16, u32, u32, *const c_char) -> bool, 0, port, port + 1, 0, 480, c"1.0.0.0".as_ptr());
    m!(gs, G, "SetModDir", fn(*const c_char) -> (), c"spacewar".as_ptr());
    m!(gs, G, "LogOnAnonymous", fn() -> (),);
    assert!(matches!(pump.wait(pipe, &[101, 102], 30), Some((101, _))), "server didn't sign in");
    let mut me = 0u64;
    m!(gs, G, "GetSteamID", fn(*mut u64) -> *mut u64, &mut me);
    println!("{} signed in as server {me}", if host { "host" } else { "join" });
    let net = generic(client, user, pipe, c"SteamNetworkingSockets012".as_ptr());
    let msgs = generic(client, user, pipe, c"SteamNetworkingMessages002".as_ptr());
    let finish = |ok: bool, what: &str| -> ! {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        m!(gs, G, "LogOff", fn() -> (),);
        std::thread::sleep(Duration::from_millis(600));
        std::process::exit(if ok { 0 } else { 1 });
    };
    let receive = |h: u32, want: &[u8]| -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            while pump.next(pipe).is_some() {}
            let mut out = [std::ptr::null_mut::<u8>(); 4];
            let n = m!(net, NS, "ReceiveMessagesOnConnection", fn(u32, *mut *mut u8, i32) -> i32, h, out.as_mut_ptr(), 4);
            for p in &out[..n.max(0) as usize] {
                if take_message(*p).2 == want {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    // 1. A connection by Steam ID.
    let (conn, peer) = if host {
        let listen = m!(net, NS, "CreateListenSocketP2P", fn(i32, i32, *const u8) -> u32, 5, 0, std::ptr::null());
        std::fs::write(id_file, me.to_string()).expect("write id file");
        let (mut conn, mut up, mut peer) = (0u32, false, 0u64);
        let end = Instant::now() + Duration::from_secs(40);
        while Instant::now() < end && !up {
            while let Some((cb, b)) = pump.next(pipe) {
                if cb != 1221 {
                    continue;
                }
                let state = i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap());
                if state == 1 && conn == 0 {
                    conn = u32::from_le_bytes(b[0..4].try_into().unwrap());
                    peer = u64::from_le_bytes(b[8 + 8..8 + 16].try_into().unwrap());
                    m!(net, NS, "AcceptConnection", fn(u32) -> i32, conn);
                }
                up |= state == 3;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !up {
            finish(false, "connection from the other server");
        }
        let _ = listen;
        (conn, peer)
    } else {
        let end = Instant::now() + Duration::from_secs(25);
        let peer = loop {
            if let Some(v) = std::fs::read_to_string(id_file).ok().and_then(|s| s.trim().parse::<u64>().ok()) {
                break v;
            }
            if Instant::now() > end {
                finish(false, "no host id");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let who = identity(peer);
        let conn = m!(net, NS, "ConnectP2P", fn(*const u8, i32, i32, *const u8) -> u32, who.as_ptr(), 5, 0, std::ptr::null());
        let mut up = false;
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end && !up {
            while let Some((cb, b)) = pump.next(pipe) {
                if cb == 1221 && u32::from_le_bytes(b[0..4].try_into().unwrap()) == conn {
                    up |= i32::from_le_bytes(b[8 + 176..8 + 180].try_into().unwrap()) == 3;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !up {
            finish(false, "connection to the other server");
        }
        (conn, peer)
    };
    println!("PASS connected, server to server (peer {peer})");
    // 2. Data both ways.
    let send = |h: u32, data: &[u8]| m!(net, NS, "SendMessageToConnection", fn(u32, *const u8, u32, i32, *mut i64) -> i32, h, data.as_ptr(), data.len() as u32, 8, std::ptr::null_mut());
    if host {
        if !receive(conn, b"ping") {
            finish(false, "data from the other server");
        }
        send(conn, b"pong");
    } else {
        send(conn, b"ping");
        if !receive(conn, b"pong") {
            finish(false, "data from the other server");
        }
    }
    println!("PASS data over the connection");
    // 3. ISteamNetworkingMessages.
    let who = identity(peer);
    if !host {
        m!(msgs, NM, "SendMessageToUser", fn(*const u8, *const u8, u32, i32, i32) -> i32, who.as_ptr(), b"msg up".as_ptr(), 6, 8, 2);
    }
    let want: &[u8] = if host { b"msg up" } else { b"msg down" };
    let (mut got, end) = (false, Instant::now() + Duration::from_secs(20));
    while Instant::now() < end && !got {
        while let Some((cb, _)) = pump.next(pipe) {
            if cb == 1251 {
                m!(msgs, NM, "AcceptSessionWithUser", fn(*const u8) -> bool, who.as_ptr());
            }
        }
        let mut out = [std::ptr::null_mut::<u8>(); 4];
        let n = m!(msgs, NM, "ReceiveMessagesOnChannel", fn(i32, *mut *mut u8, i32) -> i32, 2, out.as_mut_ptr(), 4);
        for p in &out[..n.max(0) as usize] {
            let (_, from, data) = take_message(*p);
            got |= from == peer && data == want;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if got && host {
        m!(msgs, NM, "SendMessageToUser", fn(*const u8, *const u8, u32, i32, i32) -> i32, who.as_ptr(), b"msg down".as_ptr(), 8, 8, 2);
        // Keep the session up long enough for it to arrive.
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            while pump.next(pipe).is_some() {}
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    finish(got, "ISteamNetworkingMessages, server to server");
}

/// `--test gs-api`: a dedicated server through a real `steam_api64.dll` next
/// to this exe (SteamGameServer_Init and the flat API), with our DLL beside
/// it as `steamclient64.dll`. Signs in anonymously, unlisted, then shuts down.
fn gs_api_test() -> ! {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let w: Vec<u16> = dir.join(STEAM_API).to_string_lossy().encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {STEAM_API}");
    let api = Api(module);
    const PORT: u16 = 27945;
    // Authentication mode; the second argument is the SDK's unused legacy port.
    let up = call!(api, "SteamInternal_GameServer_Init", fn(u32, u16, u16, u16, i32, *const c_char) -> bool, 0, 0, PORT, PORT + 1, 2, c"1.0.0.0".as_ptr());
    println!("SteamGameServer_Init = {up}");
    if !up {
        std::process::exit(1);
    }
    call!(api, "SteamAPI_ManualDispatch_Init", fn() -> (),);
    let pipe = call!(api, "SteamGameServer_GetHSteamPipe", fn() -> i32,);
    let gs = api.iface("SteamGameServer", &["v015", "v014", "v013"]);
    assert!(gs != 0, "no ISteamGameServer accessor");
    call!(api, "SteamAPI_ISteamGameServer_SetProduct", fn(usize, *const c_char) -> (), gs, c"SteamworksExample".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_SetGameDescription", fn(usize, *const c_char) -> (), gs, c"SteamInRust test".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_SetModDir", fn(usize, *const c_char) -> (), gs, c"spacewar".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_SetDedicatedServer", fn(usize, bool) -> (), gs, true);
    call!(api, "SteamAPI_ISteamGameServer_SetMaxPlayerCount", fn(usize, i32) -> (), gs, 2);
    call!(api, "SteamAPI_ISteamGameServer_SetServerName", fn(usize, *const c_char) -> (), gs, c"SteamInRust flat API test".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_SetMapName", fn(usize, *const c_char) -> (), gs, c"sirtest".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_LogOnAnonymous", fn(usize) -> (), gs);
    let mut signed = false;
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end && !signed {
        call!(api, "SteamAPI_ManualDispatch_RunFrame", fn(i32) -> (), pipe);
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        while call!(api, "SteamAPI_ManualDispatch_GetNextCallback", fn(i32, *mut CallbackMsg) -> bool, pipe, &mut msg) {
            println!("  callback {} on the server's pipe", msg.callback);
            signed |= msg.callback == 101;
            call!(api, "SteamAPI_ManualDispatch_FreeLastCallback", fn(i32) -> (), pipe);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let on = call!(api, "SteamAPI_ISteamGameServer_BLoggedOn", fn(usize) -> bool, gs);
    let id = call!(api, "SteamAPI_ISteamGameServer_GetSteamID", fn(usize) -> u64, gs);
    println!("{} signed in through steam_api: logged on {on}, Steam ID {id}", if signed && on && id >> 52 == 0x014 { "PASS" } else { "FAIL" });
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let info = a2s(&sock, &format!("127.0.0.1:{}", PORT + 1), b"\xFF\xFF\xFF\xFFTSource Engine Query\0").unwrap_or_default();
    let named = String::from_utf8_lossy(&info).contains("SteamInRust flat API test");
    println!("{} A2S_INFO with the name set through the flat API", if named { "PASS" } else { "FAIL" });
    call!(api, "SteamAPI_ISteamGameServer_LogOff", fn(usize) -> (), gs);
    std::thread::sleep(Duration::from_millis(500));
    call!(api, "SteamGameServer_Shutdown", fn() -> (),);
    println!("shut down");
    std::process::exit(if signed && on && named { 0 } else { 1 });
}

/// Windows: makes this process alone see a stand-in for Steam's registry
/// values (the rest of the PC, and Steam's real keys, are untouched), so
/// steam_api finds our DLL the way a game SteamInRust starts does.
#[cfg(windows)]
mod private_registry {
    use std::ffi::c_void;
    const HKCU: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_ALL_ACCESS: u32 = 0xF003F;
    const REG_SZ: u32 = 1;
    const REG_DWORD: u32 = 4;
    const ROOT: &str = "Software\\SteamInRustGameServerTest";

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(key: isize, sub: *const u16, r: u32, class: *const u16, opt: u32, sam: u32, sec: *const c_void, out: *mut isize, disp: *mut u32) -> i32;
        fn RegSetValueExW(key: isize, name: *const u16, r: u32, kind: u32, data: *const u8, size: u32) -> i32;
        fn RegCloseKey(key: isize) -> i32;
        fn RegOverridePredefKey(key: isize, new: isize) -> i32;
        fn RegDeleteTreeW(key: isize, sub: *const u16) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn create(parent: isize, sub: &str) -> isize {
        let mut key = 0;
        let r = unsafe { RegCreateKeyExW(parent, wide(sub).as_ptr(), 0, std::ptr::null(), 0, KEY_ALL_ACCESS, std::ptr::null(), &mut key, std::ptr::null_mut()) };
        assert_eq!(r, 0, "can't create the test key");
        key
    }

    pub fn install(dll: &str) {
        let root = create(HKCU, ROOT);
        let active = create(root, "Software\\Valve\\Steam\\ActiveProcess");
        let path: Vec<u8> = wide(dll).iter().flat_map(|c| c.to_le_bytes()).collect();
        for (name, kind, data) in [
            ("SteamClientDll64", REG_SZ, path),
            ("pid", REG_DWORD, std::process::id().to_le_bytes().to_vec()),
            ("ActiveUser", REG_DWORD, 1u32.to_le_bytes().to_vec()),
        ] {
            unsafe { RegSetValueExW(active, wide(name).as_ptr(), 0, kind, data.as_ptr(), data.len() as u32) };
        }
        unsafe {
            RegCloseKey(active);
            assert_eq!(RegOverridePredefKey(HKCU, root), 0);
            RegCloseKey(root);
        }
    }

    pub fn remove() {
        unsafe {
            RegOverridePredefKey(HKCU, 0);
            RegDeleteTreeW(HKCU, wide(ROOT).as_ptr());
        }
    }
}

/// `--test gs-listen --client-dll <our dll>`: a listen server, the way a game
/// runs one: real steam_api finds our DLL through (its private view of) the
/// registry, the player and the server each get their own pipe, and the
/// server checks the player's ticket with Steam.
#[cfg(windows)]
fn gs_listen_test(dll: &str) -> ! {
    private_registry::install(dll);
    let finish = |ok: bool| -> ! {
        private_registry::remove();
        std::process::exit(if ok { 0 } else { 1 });
    };
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let w: Vec<u16> = dir.join(STEAM_API).to_string_lossy().encode_utf16().chain([0]).collect();
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!module.is_null(), "can't load {STEAM_API}");
    let api = Api(module);
    let player_up = if let Some(init) = api.f("SteamAPI_InitFlat") {
        let init: extern "C" fn(*mut c_char) -> i32 = unsafe { std::mem::transmute(init) };
        let mut err = [0 as c_char; 1024];
        init(err.as_mut_ptr()) == 0
    } else {
        call!(api, "SteamAPI_Init", fn() -> bool,)
    };
    println!("{} player side (SteamAPI_Init)", if player_up { "PASS" } else { "FAIL" });
    if !player_up {
        finish(false);
    }
    call!(api, "SteamAPI_ManualDispatch_Init", fn() -> (),);
    let player_pipe = call!(api, "SteamAPI_GetHSteamPipe", fn() -> i32,);
    const PORT: u16 = 27955;
    let up = call!(api, "SteamInternal_GameServer_Init", fn(u32, u16, u16, u16, i32, *const c_char) -> bool, 0, 0, PORT, PORT + 1, 2, c"1.0.0.0".as_ptr());
    let server_pipe = call!(api, "SteamGameServer_GetHSteamPipe", fn() -> i32,);
    let separate = up && server_pipe != 0 && server_pipe != player_pipe;
    println!("{} server side has its own pipe: player {player_pipe}, server {server_pipe}", if separate { "PASS" } else { "FAIL" });
    if !separate {
        finish(false);
    }
    let gs = api.iface("SteamGameServer", &["v015", "v014", "v013"]);
    call!(api, "SteamAPI_ISteamGameServer_SetModDir", fn(usize, *const c_char) -> (), gs, c"spacewar".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_SetServerName", fn(usize, *const c_char) -> (), gs, c"SteamInRust listen test".as_ptr());
    call!(api, "SteamAPI_ISteamGameServer_LogOnAnonymous", fn(usize) -> (), gs);
    // Everything arriving on each pipe, until `done`.
    let pump = |until: &mut dyn FnMut(i32, i32, &[u8]) -> bool, secs: u64| -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            for pipe in [player_pipe, server_pipe] {
                call!(api, "SteamAPI_ManualDispatch_RunFrame", fn(i32) -> (), pipe);
                let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
                while call!(api, "SteamAPI_ManualDispatch_GetNextCallback", fn(i32, *mut CallbackMsg) -> bool, pipe, &mut msg) {
                    let d = unsafe { std::slice::from_raw_parts(msg.param, msg.size.max(0) as usize) }.to_vec();
                    let (id, _) = (msg.callback, ());
                    call!(api, "SteamAPI_ManualDispatch_FreeLastCallback", fn(i32) -> (), pipe);
                    if until(pipe, id, &d) {
                        return true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        false
    };
    let mut wrong_pipe = false;
    let signed = pump(&mut |pipe, id, _| {
        wrong_pipe |= id == 101 && pipe != server_pipe;
        id == 101 && pipe == server_pipe
    }, 30);
    let sid = call!(api, "SteamAPI_ISteamGameServer_GetSteamID", fn(usize) -> u64, gs);
    println!("{} server signed in on its own pipe: Steam ID {sid}", if signed && !wrong_pipe { "PASS" } else { "FAIL" });
    if !signed {
        finish(false);
    }
    let user = api.iface("SteamUser", &["v023", "v021", "v020"]);
    let me = call!(api, "SteamAPI_ISteamUser_GetSteamID", fn(usize) -> u64, user);
    let mut ticket = vec![0u8; 2560];
    let mut len = 0u32;
    let handle = call!(api, "SteamAPI_ISteamUser_GetAuthSessionTicket", fn(usize, *mut u8, i32, *mut u32, *const u8) -> u32, user, ticket.as_mut_ptr(), 2560, &mut len, std::ptr::null());
    let ready = pump(&mut |pipe, id, _| id == 163 && pipe == player_pipe, 20);
    let begin = call!(api, "SteamAPI_ISteamGameServer_BeginAuthSession", fn(usize, *const u8, i32, u64) -> i32, gs, ticket.as_ptr(), len as i32, me);
    let mut response = -1;
    let answered = pump(&mut |pipe, id, d| {
        if id == 143 && pipe == server_pipe {
            response = i32::from_le_bytes(d[8..12].try_into().unwrap());
            return true;
        }
        false
    }, 25);
    let authed = ready && begin == 0 && answered && response == 0;
    println!("{} player's ticket checked by the server: begin {begin}, response {response}", if authed { "PASS" } else { "FAIL" });
    call!(api, "SteamAPI_ISteamGameServer_EndAuthSession", fn(usize, u64) -> (), gs, me);
    call!(api, "SteamAPI_ISteamUser_CancelAuthTicket", fn(usize, u32) -> (), user, handle);
    call!(api, "SteamAPI_ISteamGameServer_LogOff", fn(usize) -> (), gs);
    std::thread::sleep(Duration::from_millis(500));
    call!(api, "SteamGameServer_Shutdown", fn() -> (),);
    call!(api, "SteamAPI_Shutdown", fn() -> (),);
    println!("shut down");
    finish(authed && !wrong_pipe);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let (Some(dll), Some("cert")) = (args.iter().position(|a| a == "--direct").and_then(|i| args.get(i + 1)), args.iter().position(|a| a == "--test").and_then(|i| args.get(i + 1)).map(|s| s.as_str())) {
        cert_test(dll);
    }
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    #[cfg(windows)]
    if arg("--test").as_deref() == Some("gs-listen") {
        gs_listen_test(&arg("--client-dll").expect("--client-dll"));
    }
    if arg("--test").as_deref() == Some("gs-api") {
        gs_api_test();
    }
    if let (Some(dll), Some("gs-net")) = (arg("--direct"), arg("--test").as_deref()) {
        gs_net_test(&dll, arg("--role").as_deref() == Some("host"), &arg("--id-file").expect("--id-file"));
    }
    if let (Some(dll), Some("gameserver")) = (arg("--direct"), arg("--test").as_deref()) {
        gameserver_test(&dll, arg("--port").and_then(|p| p.parse().ok()).unwrap_or(27915), args.iter().any(|a| a == "--bad-token"), args.iter().any(|a| a == "--player-net"));
    }
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
    let dll: Vec<u16> = dir.join(STEAM_API).to_string_lossy().encode_utf16().chain([0]).collect();
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

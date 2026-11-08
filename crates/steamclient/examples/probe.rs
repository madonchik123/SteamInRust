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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
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

//! C++ interface objects: a pointer to a vtable of function pointers, laid
//! out exactly like the MSVC-compiled Steamworks headers of each version.
//!
//! Layouts come from [`crate::tables`] (method names per slot). Each slot
//! gets our implementation when there is one, otherwise a stub that returns
//! zero (false / null / invalid handle), or for methods that return a struct
//! (CSteamID…) a stub that zeroes the caller's return buffer.
//!
//! 32-bit games call methods with `thiscall`: `this` in ECX, arguments on
//! the stack, and the method pops them. Our methods are plain C functions
//! taking `this` first, so on x86 each slot gets a small generated
//! trampoline that copies the arguments, calls ours, and pops the exact
//! number of bytes recorded in [`crate::tables32`].

use crate::tables::LAYOUTS;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};

pub type Fp = *const c_void;

/// ISteamClient017 (SDK 1.37–1.41), from the SDK headers: older games ask
/// for it. Only slot positions matter; getters are resolved by version.
const CLIENT017: &[&str] = &[
    "CreateSteamPipe", "BReleaseSteamPipe", "ConnectToGlobalUser", "CreateLocalUser", "ReleaseUser", "GetISteamUser",
    "GetISteamGameServer", "SetLocalIPBinding", "GetISteamFriends", "GetISteamUtils", "GetISteamMatchmaking",
    "GetISteamMatchmakingServers", "GetISteamGenericInterface", "GetISteamUserStats", "GetISteamGameServerStats",
    "GetISteamApps", "GetISteamNetworking", "GetISteamRemoteStorage", "GetISteamScreenshots", "RunFrame", "GetIPCCallCount",
    "SetWarningMessageHook", "BShutdownIfAllPipesClosed", "GetISteamHTTP", "GetISteamUnifiedMessages", "GetISteamController",
    "GetISteamUGC", "GetISteamAppList", "GetISteamMusic", "GetISteamMusicRemote", "GetISteamHTMLSurface",
    "DEPRECATED_Set_SteamAPI_CPostAPIResultInProcess", "DEPRECATED_Remove_SteamAPI_CPostAPIResultInProcess",
    "Set_SteamAPI_CCheckCallbackRegisteredInProcess", "GetISteamInventory", "GetISteamVideo", "GetISteamParentalSettings",
];

/// Interface name and slot list for a version string, falling back to the
/// nearest known version of the same interface.
pub fn layout(version: &str) -> Option<(&'static str, &'static [&'static str])> {
    if let Some(&(_, kind, methods)) = LAYOUTS.iter().find(|(v, _, _)| *v == version) {
        return Some((kind, methods));
    }
    let (prefix, num) = split_version(version);
    if prefix == "SteamClient" && num <= 17 {
        return Some(("ISteamClient", CLIENT017));
    }
    // Same interface, other version: newest one not newer than asked, else
    // the oldest newer one. Steam mostly appends methods between versions.
    let same: Vec<(u32, &'static str, &'static [&'static str])> =
        LAYOUTS.iter().filter(|(v, _, _)| split_version(v).0 == prefix).map(|(v, k, m)| (split_version(v).1, *k, *m)).collect();
    let pick = same.iter().filter(|(n, _, _)| *n <= num).max_by_key(|(n, _, _)| *n).or_else(|| same.iter().min_by_key(|(n, _, _)| *n));
    pick.map(|(_, k, m)| (*k, *m))
}

/// Stack size not known for a slot.
pub const UNKNOWN: u16 = 0xFFFF;

/// 32-bit layout: interface name and (method, bytes the method pops) per
/// slot. Versions the 32-bit tables don't have use the 64-bit slot names
/// with sizes looked up by method name.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub fn layout32(version: &str) -> Option<(&'static str, Vec<(&'static str, u16)>)> {
    use crate::tables32::LAYOUTS32;
    if let Some(&(_, kind, methods)) = LAYOUTS32.iter().find(|(v, _, _)| *v == version) {
        return Some((kind, methods.to_vec()));
    }
    let (kind, names) = layout(version)?;
    let sizes: HashMap<&str, u16> = LAYOUTS32.iter().filter(|(_, k, _)| *k == kind).flat_map(|(_, _, m)| m.iter().copied()).collect();
    Some((kind, names.iter().map(|n| (*n, sizes.get(n).copied().or_else(|| client_bytes(kind, n)).unwrap_or(UNKNOWN))).collect()))
}

/// ISteamClient methods by name (older versions have other slot orders).
fn client_bytes(kind: &str, name: &str) -> Option<u16> {
    if kind != "ISteamClient" {
        return None;
    }
    Some(match name {
        "GetISteamUtils" => 8,
        n if n.starts_with("GetISteam") => 12,
        "CreateSteamPipe" | "RunFrame" | "GetIPCCallCount" | "BShutdownIfAllPipesClosed" | "DestroyAllInterfaces" => 0,
        "BReleaseSteamPipe"
        | "ConnectToGlobalUser"
        | "SetWarningMessageHook"
        | "Set_SteamAPI_CCheckCallbackRegisteredInProcess"
        | "DEPRECATED_Set_SteamAPI_CPostAPIResultInProcess"
        | "DEPRECATED_Remove_SteamAPI_CPostAPIResultInProcess" => 4,
        "CreateLocalUser" | "ReleaseUser" | "SetLocalIPBinding" => 8,
        _ => return None,
    })
}

#[cfg(target_arch = "x86")]
mod x86 {
    use super::Fp;
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    }

    const SIZE: usize = 48;

    /// thiscall -> cdecl(this, args...) -> `ret bytes`, one per slot.
    pub fn trampolines(targets: &[(Fp, u16)]) -> Vec<Fp> {
        // MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE
        let mem = unsafe { VirtualAlloc(std::ptr::null_mut(), targets.len() * SIZE, 0x3000, 0x40) } as *mut u8;
        if mem.is_null() {
            return targets.iter().map(|_| super::zero as Fp).collect();
        }
        targets
            .iter()
            .enumerate()
            .map(|(i, &(target, bytes))| {
                let code = code(target as u32, bytes);
                unsafe {
                    let at = mem.add(i * SIZE);
                    std::ptr::copy_nonoverlapping(code.as_ptr(), at, code.len());
                    at as Fp
                }
            })
            .collect()
    }

    fn code(target: u32, bytes: u16) -> Vec<u8> {
        let k = bytes as u32;
        let mut c = vec![0x55, 0x89, 0xE5, 0x56, 0x57, 0x51]; // push ebp; mov ebp,esp; push esi; push edi; push ecx
        c.extend_from_slice(&[0x81, 0xEC]); // sub esp, k
        c.extend_from_slice(&k.to_le_bytes());
        c.extend_from_slice(&[0x89, 0xE7, 0x8D, 0x75, 0x08, 0xB9]); // mov edi,esp; lea esi,[ebp+8]; mov ecx, k/4
        c.extend_from_slice(&(k / 4).to_le_bytes());
        c.extend_from_slice(&[0xF3, 0xA5, 0xFF, 0x75, 0xF4, 0xB8]); // rep movsd; push [ebp-12] (this); mov eax, target
        c.extend_from_slice(&target.to_le_bytes());
        c.extend_from_slice(&[0xFF, 0xD0, 0x8D, 0x65, 0xF8, 0x5F, 0x5E, 0x5D, 0xC2]); // call eax; lea esp,[ebp-8]; pop edi; pop esi; pop ebp; ret k
        c.extend_from_slice(&bytes.to_le_bytes());
        c
    }

    #[cfg(test)]
    mod tests {
        type F = unsafe extern "thiscall" fn(*mut u8, u32, u64, f32) -> u64;

        extern "C" fn sum(this: *mut u8, a: u32, b: u64, c: f32) -> u64 {
            this as u64 + a as u64 + b + c as u64
        }

        #[test]
        fn thiscall_reaches_cdecl() {
            let t = super::trampolines(&[(sum as super::Fp, 16)]);
            let f: F = unsafe { std::mem::transmute(t[0]) };
            assert_eq!(unsafe { f(100 as *mut u8, 1, 1 << 40, 2.0) }, 100 + 1 + (1 << 40) + 2);
        }
    }
}

fn split_version(v: &str) -> (&str, u32) {
    let digits = v.len() - v.bytes().rev().take_while(|b| b.is_ascii_digit()).count();
    (&v[..digits], v[digits..].parse().unwrap_or(0))
}

#[repr(C)]
struct Object {
    vtable: *const Fp,
}

static OBJECTS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The interface object for a version string (built once, kept forever).
pub fn get(version: &str) -> *mut c_void {
    let mut objects = OBJECTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&p) = objects.get(version) {
        return p as *mut c_void;
    }
    #[cfg(not(target_arch = "x86"))]
    let slots: Vec<Fp> = match layout(version) {
        Some((kind, methods)) => {
            crate::log!("interface {version} ({kind}, {} methods)", methods.len());
            methods.iter().map(|m| crate::ifaces::imp(kind, m).unwrap_or_else(|| stub(kind, m))).collect()
        }
        None => {
            crate::log!("interface {version}: unknown");
            return std::ptr::null_mut();
        }
    };
    #[cfg(target_arch = "x86")]
    let slots: Vec<Fp> = match layout32(version) {
        Some((kind, methods)) => {
            crate::log!("interface {version} ({kind}, {} methods, 32-bit)", methods.len());
            let targets: Vec<(Fp, u16)> = methods
                .iter()
                .map(|(m, bytes)| match *bytes {
                    UNKNOWN => (zero as Fp, 0),
                    b => (crate::ifaces::imp(kind, m).unwrap_or_else(|| stub(kind, m)), b),
                })
                .collect();
            x86::trampolines(&targets)
        }
        None => {
            crate::log!("interface {version}: unknown");
            return std::ptr::null_mut();
        }
    };
    // Spare zero slots after the known ones, for methods a newer SDK appended.
    let mut slots = slots;
    #[cfg(not(target_arch = "x86"))]
    slots.extend(std::iter::repeat_n(zero as Fp, 64));
    #[cfg(target_arch = "x86")]
    slots.extend(x86::trampolines(&[(zero as Fp, 0); 64]));
    let vtable = Box::leak(slots.into_boxed_slice()).as_ptr();
    let obj = Box::leak(Box::new(Object { vtable })) as *mut Object as usize;
    objects.insert(version.to_string(), obj);
    obj as *mut c_void
}

/// Methods returning a struct by value get a hidden return pointer (MSVC
/// does that for every member function returning a class). Size in bytes.
fn struct_return(kind: &str, name: &str) -> Option<usize> {
    Some(match (kind, name) {
        (_, "GetSteamID" | "GetAppOwner" | "GetFriendByIndex" | "GetClanByIndex" | "GetFriendFromSourceByIndex" | "GetClanOwner"
        | "GetClanOfficerByIndex" | "GetCoplayFriend" | "GetChatMemberByIndex" | "GetLobbyByIndex" | "GetLobbyMemberByIndex"
        | "GetLobbyOwner" | "GetSessionSteamID") => 8,
        ("ISteamGameServer", "GetPublicIP") => 20,
        ("ISteamInput" | "ISteamController", "GetDigitalActionData") => 2,
        ("ISteamInput" | "ISteamController", "GetAnalogActionData") => 16,
        ("ISteamInput" | "ISteamController", "GetMotionData") => 40,
        _ => return None,
    })
}

fn stub(kind: &str, name: &str) -> Fp {
    match struct_return(kind, name) {
        Some(2) => sret::<2> as Fp,
        Some(8) => sret::<8> as Fp,
        Some(16) => sret::<16> as Fp,
        Some(20) => sret::<20> as Fp,
        Some(40) => sret::<40> as Fp,
        _ => zero as Fp,
    }
}

/// Returns 0 in every register a caller could read the result from.
pub extern "C" fn zero() -> usize {
    0
}

extern "C" fn sret<const N: usize>(_this: *mut c_void, ret: *mut u8) -> *mut u8 {
    if !ret.is_null() {
        unsafe { std::ptr::write_bytes(ret, 0, N) };
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_layouts() {
        let (kind, m) = layout("SteamUser023").unwrap();
        assert_eq!(kind, "ISteamUser");
        assert_eq!(m[2], "GetSteamID");
        // MSVC puts overloads in reverse order.
        let (_, m) = layout("STEAMUSERSTATS_INTERFACE_VERSION013").unwrap();
        assert_eq!(&m[..2], &["GetStatFloat", "GetStatInt32"]);
        // Unknown versions fall back to a neighbour of the same interface.
        assert_eq!(layout("SteamUser020").unwrap().1[2], "GetSteamID");
        assert_eq!(layout("SteamFriends015").unwrap().0, "ISteamFriends");
        assert_eq!(layout("SteamClient017").unwrap().1.len(), CLIENT017.len());
        assert!(layout("NotAnInterface001").is_none());
    }

    #[test]
    fn layouts_32bit() {
        // From 32-bit steam_api.dll thunks: hidden return pointer counted.
        let (kind, m) = layout32("SteamUser021").unwrap();
        assert_eq!(kind, "ISteamUser");
        assert_eq!(m[2], ("GetSteamID", 4));
        let (_, m) = layout32("SteamNetworking006").unwrap();
        assert_eq!(m[0], ("SendP2PPacket", 24));
        let (_, m) = layout32("SteamMatchMaking009").unwrap();
        assert!(m.contains(&("GetLobbyMemberByIndex", 16)));
        // A version only the 64-bit tables have: sizes by method name.
        let (_, m) = layout32("SteamUser023").unwrap();
        assert_eq!(m[1], ("BLoggedOn", 0));
        for (v, _, methods) in crate::tables32::LAYOUTS32 {
            assert!(methods.iter().all(|(_, b)| b % 4 == 0 && *b < 256), "{v}");
        }
        assert_eq!(layout32("SteamClient017").unwrap().1[9], ("GetISteamUtils", 8));
        // Every ISteamClient017 slot needs its real size: a 32-bit stub that
        // pops the wrong number of bytes breaks the game's stack (Human Fall
        // Flat crashed asking for GetISteamUnifiedMessages, slot 24).
        let (_, m) = layout32("SteamClient017").unwrap();
        for (i, (name, bytes)) in m.iter().enumerate() {
            assert!(*bytes != UNKNOWN, "SteamClient017 slot {i} ({name:?}) has no size");
        }
        assert_eq!(m[24], ("GetISteamUnifiedMessages", 12));
        // Old versions have their own exact layouts (not a newer one's).
        let (_, m) = layout32("SteamUser019").unwrap();
        assert_eq!(m[2], ("GetSteamID", 4));
        let (_, m) = layout32("SteamController005").unwrap();
        assert_eq!(m[12], ("GetAnalogActionData", 20));
        let (_, m) = layout32("STEAMUSERSTATS_INTERFACE_VERSION011").unwrap();
        assert_eq!(&m[1..3], &[("GetStatFloat", 8), ("GetStatInt32", 8)]);
    }

    #[test]
    fn every_layout_slot_resolves() {
        for (v, kind, methods) in LAYOUTS {
            assert!(!methods.is_empty(), "{v}");
            for m in *methods {
                let _ = crate::ifaces::imp(kind, m).unwrap_or_else(|| stub(kind, m));
            }
        }
    }
}

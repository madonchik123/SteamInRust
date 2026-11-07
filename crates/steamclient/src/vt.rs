//! C++ interface objects: a pointer to a vtable of function pointers, laid
//! out exactly like the MSVC-compiled Steamworks headers of each version.
//!
//! Layouts come from [`crate::tables`] (method names per slot). Each slot
//! gets our implementation when there is one, otherwise a stub that returns
//! zero (false / null / invalid handle), or for methods that return a struct
//! (CSteamID…) a stub that zeroes the caller's return buffer.

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
    "GetISteamApps", "GetISteamNetworking", "GetISteamRemoteStorage", "GetISteamScreenshots", "", "GetIPCCallCount",
    "SetWarningMessageHook", "BShutdownIfAllPipesClosed", "GetISteamHTTP", "", "GetISteamController", "GetISteamUGC",
    "GetISteamAppList", "GetISteamMusic", "GetISteamMusicRemote", "GetISteamHTMLSurface", "", "", "",
    "GetISteamInventory", "GetISteamVideo", "GetISteamParentalSettings",
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
    // Spare zero slots after the known ones, for methods a newer SDK appended.
    let mut slots = slots;
    slots.extend(std::iter::repeat_n(zero as Fp, 64));
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
    fn every_layout_slot_resolves() {
        for (v, kind, methods) in LAYOUTS {
            assert!(!methods.is_empty(), "{v}");
            for m in *methods {
                let _ = crate::ifaces::imp(kind, m).unwrap_or_else(|| stub(kind, m));
            }
        }
    }
}

//! Whether the push-to-talk key is held, in any app.
//!
//! This asks the system about that one key while a call is on. It doesn't
//! watch other keys, and the key still reaches whatever app is in front.

/// Every key that can be chosen, by the name saved in settings.
pub const KEYS: &[&str] = &[
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20",
    "Space", "Tab", "CapsLock", "Backquote", "LeftCtrl", "RightCtrl", "LeftShift", "RightShift", "LeftAlt", "RightAlt", "MouseMiddle", "Mouse4", "Mouse5",
];

/// Is this key a real choice?
pub fn known(key: &str) -> bool {
    KEYS.contains(&key)
}

/// Is the key down right now? Unknown keys never are.
pub fn is_down(key: &str) -> bool {
    platform::is_down(key)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn virtual_key(key: &str) -> Option<i32> {
    Some(match key {
        "Space" => 0x20,
        "Tab" => 0x09,
        "CapsLock" => 0x14,
        "Backquote" => 0xC0,
        "LeftCtrl" => 0xA2,
        "RightCtrl" => 0xA3,
        "LeftShift" => 0xA0,
        "RightShift" => 0xA1,
        "LeftAlt" => 0xA4,
        "RightAlt" => 0xA5,
        "MouseMiddle" => 0x04,
        "Mouse4" => 0x05,
        "Mouse5" => 0x06,
        k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => k.as_bytes()[0].to_ascii_uppercase() as i32,
        k => {
            let n: i32 = k.strip_prefix('F')?.parse().ok()?;
            if !(1..=20).contains(&n) {
                return None;
            }
            0x70 + n - 1
        }
    })
}

#[cfg(windows)]
mod platform {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    pub fn is_down(key: &str) -> bool {
        super::virtual_key(key).is_some_and(|vk| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0)
    }
}

#[cfg(target_vendor = "apple")]
mod platform {
    const COMBINED_SESSION: i32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    }

    fn key_code(key: &str) -> Option<u16> {
        Some(match key {
            "A" => 0,
            "S" => 1,
            "D" => 2,
            "F" => 3,
            "H" => 4,
            "G" => 5,
            "Z" => 6,
            "X" => 7,
            "C" => 8,
            "V" => 9,
            "B" => 11,
            "Q" => 12,
            "W" => 13,
            "E" => 14,
            "R" => 15,
            "Y" => 16,
            "T" => 17,
            "1" => 18,
            "2" => 19,
            "3" => 20,
            "4" => 21,
            "6" => 22,
            "5" => 23,
            "9" => 25,
            "7" => 26,
            "8" => 28,
            "0" => 29,
            "O" => 31,
            "U" => 32,
            "I" => 34,
            "P" => 35,
            "L" => 37,
            "J" => 38,
            "K" => 40,
            "N" => 45,
            "M" => 46,
            "Tab" => 48,
            "Space" => 49,
            "Backquote" => 50,
            "LeftShift" => 56,
            "CapsLock" => 57,
            "LeftAlt" => 58,
            "LeftCtrl" => 59,
            "RightShift" => 60,
            "RightAlt" => 61,
            "RightCtrl" => 62,
            "F17" => 64,
            "F18" => 79,
            "F19" => 80,
            "F20" => 90,
            "F5" => 96,
            "F6" => 97,
            "F7" => 98,
            "F3" => 99,
            "F8" => 100,
            "F9" => 101,
            "F11" => 103,
            "F13" => 105,
            "F16" => 106,
            "F14" => 107,
            "F10" => 109,
            "F12" => 111,
            "F15" => 113,
            "F4" => 118,
            "F2" => 120,
            "F1" => 122,
            _ => return None,
        })
    }

    pub fn is_down(key: &str) -> bool {
        let button = match key {
            "MouseMiddle" => Some(2),
            "Mouse4" => Some(3),
            "Mouse5" => Some(4),
            _ => None,
        };
        unsafe {
            match button {
                Some(b) => CGEventSourceButtonState(COMBINED_SESSION, b),
                None => key_code(key).is_some_and(|k| CGEventSourceKeyState(COMBINED_SESSION, k)),
            }
        }
    }
}

#[cfg(not(any(windows, target_vendor = "apple")))]
mod platform {
    pub fn is_down(_key: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_code() {
        for k in KEYS {
            assert!(virtual_key(k).is_some(), "{k}");
        }
        assert_eq!(virtual_key("A"), Some(0x41));
        assert_eq!(virtual_key("7"), Some(0x37));
        assert_eq!(virtual_key("F1"), Some(0x70));
        assert_eq!(virtual_key("F20"), Some(0x83));
        assert_eq!(virtual_key("F21"), None);
        assert_eq!(virtual_key("Nope"), None);
    }

    #[test]
    fn an_unknown_key_is_never_held() {
        assert!(!known("Nope"));
        assert!(!is_down("Nope"));
    }
}

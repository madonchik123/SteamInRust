//! Steam result codes. Only the ones users are likely to see get friendly text.

pub const OK: i32 = 1;
pub const FAIL: i32 = 2;
pub const INVALID_PASSWORD: i32 = 5;
pub const LOGGED_IN_ELSEWHERE: i32 = 6;
pub const SERVICE_UNAVAILABLE: i32 = 20;
pub const EXPIRED: i32 = 27;
pub const TRY_ANOTHER_CM: i32 = 48;
pub const ACCOUNT_LOGON_DENIED: i32 = 63;
pub const INVALID_LOGIN_AUTH_CODE: i32 = 65;
pub const RATE_LIMIT_EXCEEDED: i32 = 84;
pub const NEED_TWO_FACTOR: i32 = 85;
pub const LOGIN_THROTTLED: i32 = 87;
pub const TWO_FACTOR_MISMATCH: i32 = 88;
pub const ACCESS_DENIED: i32 = 15;
pub const LOGON_SESSION_REPLACED: i32 = 34;

pub fn describe(code: i32, detail: &str) -> String {
    let text = match code {
        OK => "OK",
        FAIL => "Steam reported a failure",
        INVALID_PASSWORD => "Incorrect account name or password",
        LOGGED_IN_ELSEWHERE => "Logged in from another location",
        SERVICE_UNAVAILABLE => "Steam is temporarily unavailable",
        EXPIRED => "The login session expired, try again",
        TRY_ANOTHER_CM => "Server busy, try again",
        ACCOUNT_LOGON_DENIED => "Steam Guard code required",
        INVALID_LOGIN_AUTH_CODE => "Incorrect Steam Guard email code",
        RATE_LIMIT_EXCEEDED => "Too many attempts, wait a while and try again",
        NEED_TWO_FACTOR => "Steam Guard mobile code required",
        LOGIN_THROTTLED => "Too many login attempts, wait a while",
        TWO_FACTOR_MISMATCH => "Incorrect Steam Guard mobile code",
        ACCESS_DENIED => "Access denied (saved login may have been revoked)",
        LOGON_SESSION_REPLACED => "Signed out: this account signed in again somewhere else with the same session. Click your account to sign back in.",
        _ => "",
    };
    match (text.is_empty(), detail.is_empty()) {
        (true, true) => format!("Steam error {code}"),
        (true, false) => format!("Steam error {code}: {detail}"),
        (false, true) => text.to_string(),
        (false, false) => format!("{text} ({detail})"),
    }
}

//! Message type ids used by the client. Values come from `enums_clientserver.proto`.

pub const MULTI: u32 = 1;
pub const SERVICE_METHOD: u32 = 146;
pub const SERVICE_METHOD_RESPONSE: u32 = 147;
pub const SERVICE_METHOD_CALL_FROM_CLIENT: u32 = 151;
pub const CLIENT_HEARTBEAT: u32 = 703;
pub const CLIENT_LOG_OFF: u32 = 706;
pub const CLIENT_CHANGE_STATUS: u32 = 716;
pub const CLIENT_GAMES_PLAYED: u32 = 742;
pub const CLIENT_LOGON_RESPONSE: u32 = 751;
pub const CLIENT_LOGGED_OFF: u32 = 757;
pub const CLIENT_PERSONA_STATE: u32 = 766;
pub const CLIENT_FRIENDS_LIST: u32 = 767;
pub const CLIENT_ACCOUNT_INFO: u32 = 768;
pub const CLIENT_LICENSE_LIST: u32 = 780;
pub const CLIENT_REQUEST_FRIEND_DATA: u32 = 815;
pub const CLIENT_LOGON: u32 = 5514;
pub const CLIENT_WALLET_INFO_UPDATE: u32 = 5528;
pub const CLIENT_GET_DEPOT_DECRYPTION_KEY: u32 = 5438;
/// Add a free game to the account (answered with 5573).
pub const CLIENT_REQUEST_FREE_LICENSE: u32 = 5572;
pub const CLIENT_EMAIL_ADDR_INFO: u32 = 5456;
pub const CLIENT_ADD_FRIEND: u32 = 791;
pub const CLIENT_REMOVE_FRIEND: u32 = 714;
/// Your nicknames for friends (sent at sign-in and when they change).
pub const CLIENT_PLAYER_NICKNAME_LIST: u32 = 5587;
pub const AM_CLIENT_SET_PLAYER_NICKNAME: u32 = 5588;
pub const CLIENT_FRIEND_PROFILE_INFO: u32 = 5535;
pub const CLIENT_PICS_PRODUCT_INFO_REQUEST: u32 = 8903;
pub const CLIENT_PICS_PRODUCT_INFO_RESPONSE: u32 = 8904;
pub const CLIENT_PICS_ACCESS_TOKEN_REQUEST: u32 = 8905;
pub const CLIENT_PICS_ACCESS_TOKEN_RESPONSE: u32 = 8906;
pub const SERVICE_METHOD_CALL_FROM_CLIENT_NON_AUTHED: u32 = 9804;
pub const CLIENT_HELLO: u32 = 9805;

/// Human-readable name for logs, using the generated enum.
pub fn name(emsg: u32) -> String {
    proto::EMsg::try_from(emsg as i32)
        .map(|e| e.as_str_name().trim_start_matches("k_EMsg").to_string())
        .unwrap_or_else(|_| format!("EMsg({emsg})"))
}

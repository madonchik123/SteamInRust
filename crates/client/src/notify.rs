//! Steam notifications (the bell). Each one carries a small JSON blob whose
//! fields depend on its type, so details are picked out by common names.

use serde_json::Value;

pub fn convert(n: &proto::webui::SteamNotificationData) -> model::Notification {
    let data = n.body_data().to_string();
    tracing::debug!("notification type {} data {}", n.notification_type(), data);
    let v: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
    model::Notification {
        id: n.notification_id(),
        kind: n.notification_type(),
        time: n.timestamp() as u64,
        read: n.read(),
        app_id: find_num(&v, &["appid", "app_id"]).map(|a| a as model::AppId),
        from: find_num(&v, &["steamid", "steam_id", "requestor_id", "sender", "steamid_sender", "owner_steam_id", "accountid", "account_id"]).map(to_steamid),
        text: find_str(&v, &["title", "text", "item_name", "name", "game_name", "subject"]),
        context_id: find_num(&v, &["context_id", "contextid"]),
        asset_id: find_num(&v, &["asset_id", "assetid"]),
        source_app: find_num(&v, &["source_appid"]).map(|a| a as model::AppId),
        ticket: find_str(&v, &["ticket"]),
        data,
    }
}

/// Account ids (32-bit) become full SteamIDs.
fn to_steamid(id: u64) -> u64 {
    if id < (1 << 32) { id + 76561197960265728 } else { id }
}

fn find_num(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| {
        let x = v.get(*k)?;
        x.as_u64().or_else(|| x.as_str()?.parse().ok())
    })
    .filter(|n| *n != 0)
}

fn find_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k)?.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty())
}

//! The store's discovery queue: a dozen games picked for you, gone through
//! one at a time, like Steam's.

use model::AppId;
use net::Connection;

/// Your discovery queue (Steam keeps it until you've been through it; with
/// `rebuild` you get a fresh one).
pub async fn queue(conn: &Connection, country: &str, rebuild: bool) -> Result<Vec<AppId>, String> {
    if rebuild {
        // Like going through the queue on Steam: the games in it count as
        // seen, so the new set doesn't hand the same ones back.
        for app_id in fetch(conn, country, false).await.unwrap_or_default() {
            let _ = skip(conn, app_id).await;
        }
    }
    fetch(conn, country, rebuild).await
}

async fn fetch(conn: &Connection, country: &str, rebuild: bool) -> Result<Vec<AppId>, String> {
    let req = proto::CStoreGetDiscoveryQueueRequest {
        queue_type: Some(proto::EStoreDiscoveryQueueType::KEStoreDiscoveryQueueTypeNew as i32),
        country_code: (!country.is_empty()).then(|| country.to_string()),
        rebuild_queue: Some(rebuild),
        rebuild_queue_if_stale: Some(true),
        ..Default::default()
    };
    let r: proto::CStoreGetDiscoveryQueueResponse = conn.call("Store.GetDiscoveryQueue#1", &req).await.map_err(|e| e.to_string())?;
    Ok(r.appids)
}

/// Mark a game in the queue as seen (Steam's "Next in queue").
pub async fn skip(conn: &Connection, app_id: AppId) -> Result<(), String> {
    let req = proto::CStoreSkipDiscoveryQueueItemRequest {
        queue_type: Some(proto::EStoreDiscoveryQueueType::KEStoreDiscoveryQueueTypeNew as i32),
        appid: Some(app_id),
        ..Default::default()
    };
    conn.call::<_, proto::CStoreSkipDiscoveryQueueItemResponse>("Store.SkipDiscoveryQueueItem#1", &req).await.map(|_| ()).map_err(|e| e.to_string())
}

/// "Not interested" (or undo it): Steam stops recommending the game.
pub async fn ignore(web: &crate::market::Web, app_id: AppId, ignore: bool) -> Result<(), String> {
    let r = web
        .http
        .post("https://store.steampowered.com/recommended/ignorerecommendation/")
        .form(&[("sessionid", web.session_id.clone()), ("appid", app_id.to_string()), ("remove", if ignore { "0" } else { "1" }.to_string()), ("snr", "1_4_4__100".to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Steam answered {}", r.status()));
    }
    Ok(())
}

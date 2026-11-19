//! The wishlist's order ("Your rank"), the way Steam's wishlist page keeps
//! it: each game has a priority (1 is the top; 0 means not ranked yet, so
//! it goes after the ranked ones, oldest first). Moving a game renumbers
//! the list and sends Steam only the priorities that changed.

use model::AppId;
use serde_json::json;

/// Where Steam's wishlist page saves changes (its "Reorder" action).
const ACTION_URL: &str = "https://store.steampowered.com/wishlist/action";

/// One wishlisted game as Steam keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ranked {
    pub app_id: AppId,
    /// 1 is the top; 0 is not ranked yet.
    pub priority: u32,
    /// When it was added (unix seconds).
    pub added: u64,
}

/// Steam's order: by priority (unranked last), then oldest first, then by app.
pub fn sort(items: &mut [Ranked]) {
    items.sort_by_key(|i| (if i.priority == 0 { u32::MAX } else { i.priority }, i.added, i.app_id));
}

/// Move `app_id` to `to` (0-based, in rank order) and number the list 1, 2,
/// 3…; returns the games whose priority changed, with their new one (the
/// only ones Steam needs to hear about). Empty when nothing moves.
pub fn move_updates(ranked: &[Ranked], app_id: AppId, to: usize) -> Vec<(AppId, u32)> {
    let mut order = ranked.to_vec();
    sort(&mut order);
    let Some(from) = order.iter().position(|i| i.app_id == app_id) else { return Vec::new() };
    let to = to.min(order.len() - 1);
    if from == to {
        return Vec::new();
    }
    let item = order.remove(from);
    order.insert(to, item);
    order.iter().enumerate().filter(|(i, r)| r.priority != *i as u32 + 1).map(|(i, r)| (r.app_id, i as u32 + 1)).collect()
}

/// The request body Steam's page sends: `{"m":"Reorder","mp":[[{appid, priority}, …]]}`.
pub fn reorder_body(updates: &[(AppId, u32)]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = updates.iter().map(|(app, priority)| json!({ "appid": app, "priority": priority })).collect();
    json!({ "m": "Reorder", "mp": [list] })
}

/// Save new priorities on Steam, as the wishlist page does.
pub async fn reorder(web: &crate::market::Web, updates: &[(AppId, u32)]) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    let r = web
        .http
        .post(ACTION_URL)
        .header("x-valve-request-type", "mutationAction")
        .json(&reorder_body(updates))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Steam answered {}", r.status()));
    }
    let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    match v.pointer("/data/result").and_then(|r| r.as_i64()) {
        Some(1) => Ok(()),
        other => Err(format!("Steam didn't save the order ({})", other.map(|r| r.to_string()).unwrap_or_else(|| "no answer".into()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(app_id: AppId, priority: u32, added: u64) -> Ranked {
        Ranked { app_id, priority, added }
    }

    #[test]
    fn sorts_like_steam() {
        let mut v = vec![r(30, 0, 5), r(10, 2, 9), r(20, 1, 9), r(40, 0, 1)];
        sort(&mut v);
        assert_eq!(v.iter().map(|i| i.app_id).collect::<Vec<_>>(), [20, 10, 40, 30]);
    }

    #[test]
    fn moving_sends_only_what_changed() {
        let list = [r(1, 1, 0), r(2, 2, 0), r(3, 3, 0), r(4, 4, 0)];
        // The last one to the top: everything shifts down by one.
        assert_eq!(move_updates(&list, 4, 0), [(4, 1), (1, 2), (2, 3), (3, 4)]);
        // Swapping the middle two touches just them.
        assert_eq!(move_updates(&list, 2, 2), [(3, 2), (2, 3)]);
        // Staying put, an unknown game: nothing to send.
        assert!(move_updates(&list, 2, 1).is_empty());
        assert!(move_updates(&list, 99, 0).is_empty());
        // Past the end goes last.
        assert_eq!(move_updates(&list, 1, 50), [(2, 1), (3, 2), (4, 3), (1, 4)]);
    }

    #[test]
    fn unranked_games_get_numbers() {
        // Two not ranked yet: moving one up numbers the rest too.
        let list = [r(1, 1, 0), r(2, 0, 10), r(3, 0, 20)];
        assert_eq!(move_updates(&list, 3, 1), [(3, 2), (2, 3)]);
    }

    #[test]
    fn body_matches_the_wishlist_page() {
        let body = reorder_body(&[(1245620, 1), (367520, 2)]);
        assert_eq!(body.to_string(), r#"{"m":"Reorder","mp":[[{"appid":1245620,"priority":1},{"appid":367520,"priority":2}]]}"#);
    }
}

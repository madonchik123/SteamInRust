//! Your side of the store: which curators you follow, what they recommend,
//! exactly what you own, following a curator, and bundles in the cart. The
//! same requests Steam's own store pages make.

use crate::market::Web;
use model::AppId;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const USER_DATA: &str = "https://store.steampowered.com/dynamicstore/userdata/";
const FOLLOW: &str = "https://store.steampowered.com/curators/ajaxfollow/";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreUser {
    /// Curators you follow.
    pub followed: Vec<u32>,
    /// Per game, the curators you follow who recommend it.
    pub recommends: HashMap<AppId, Vec<u32>>,
    pub owned_apps: HashSet<AppId>,
    pub owned_packages: HashSet<u32>,
}

/// Steam's `userdata`. Its maps come back as `[]` when empty, and a curation
/// of 0 is "recommended" (as Steam's store script reads it).
pub fn parse_user(v: &Value) -> StoreUser {
    let numbers = |key: &str| -> Vec<u32> {
        v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok())).collect()).unwrap_or_default()
    };
    let mut followed: Vec<u32> = v.get("rgCurators").and_then(Value::as_object).map(|m| m.keys().filter_map(|k| k.parse().ok()).collect()).unwrap_or_default();
    followed.sort_unstable();
    let mut recommends = HashMap::new();
    if let Some(apps) = v.get("rgCurations").and_then(Value::as_object) {
        for (app, curators) in apps {
            let (Ok(app), Some(curators)) = (app.parse::<AppId>(), curators.as_object()) else { continue };
            let mut who: Vec<u32> = curators.iter().filter(|(_, kind)| kind.as_i64() == Some(0)).filter_map(|(c, _)| c.parse().ok()).collect();
            who.sort_unstable();
            if !who.is_empty() {
                recommends.insert(app, who);
            }
        }
    }
    StoreUser { followed, recommends, owned_apps: numbers("rgOwnedApps").into_iter().collect(), owned_packages: numbers("rgOwnedPackages").into_iter().collect() }
}

pub async fn load(web: &Web) -> Result<StoreUser, String> {
    let r = web.http.get(USER_DATA).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Steam answered {}", r.status()));
    }
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(parse_user(&v))
}

/// The fields of Steam's follow request.
pub fn follow_fields(session_id: &str, clan: u32, follow: bool) -> [(&'static str, String); 3] {
    [("clanid", clan.to_string()), ("sessionid", session_id.to_string()), ("follow", if follow { "1" } else { "0" }.to_string())]
}

/// Whether Steam took it. A page instead of an answer means it didn't (a
/// sign-in page, say).
pub fn follow_answer(body: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| "Steam didn't confirm that.".to_string())?;
    let Some(s) = v.get("success") else { return Ok(()) };
    let s = s.get("success").unwrap_or(s);
    if s.as_i64() == Some(1) || s.as_bool() == Some(true) {
        Ok(())
    } else {
        Err("Steam didn't confirm that.".to_string())
    }
}

pub async fn follow(web: &Web, clan: u32, follow: bool) -> Result<(), String> {
    let mut form = reqwest::multipart::Form::new();
    for (k, v) in follow_fields(&web.session_id, clan, follow) {
        form = form.text(k, v);
    }
    let r = web.http.post(FOLLOW).multipart(form).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Steam answered {}", r.status()));
    }
    follow_answer(&r.text().await.map_err(|e| e.to_string())?)
}

/// A bundle as a cart item.
pub fn bundle_cart_item(bundle_id: u32, gift: bool) -> proto::webui::CAccountCartAddItemsToCartRequestItemToAdd {
    proto::webui::CAccountCartAddItemsToCartRequestItemToAdd {
        bundleid: Some(bundle_id),
        flags: Some(proto::webui::AccountCartLineItemFlags { is_gift: Some(gift), is_private: None }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_who_you_follow_and_what_they_recommend() {
        let u = parse_user(&json!({
            "rgCurators": { "1850": { "name": "PC Gamer" }, "33": {} },
            "rgCurations": { "620": { "1850": 0, "33": 2 }, "440": { "33": 1 }, "bad": {} },
            "rgOwnedApps": [620, 440],
            "rgOwnedPackages": [7877],
        }));
        assert_eq!(u.followed, [33, 1850]);
        // Only "recommended" (0) counts.
        assert_eq!(u.recommends.get(&620).map(Vec::as_slice), Some(&[1850][..]));
        assert!(!u.recommends.contains_key(&440));
        assert!(u.owned_apps.contains(&440) && u.owned_packages.contains(&7877));
    }

    #[test]
    fn steams_empty_maps_are_lists() {
        assert_eq!(parse_user(&json!({ "rgCurators": [], "rgCurations": [], "rgOwnedApps": [] })), StoreUser::default());
    }

    #[test]
    fn the_follow_request() {
        assert_eq!(follow_fields("abc", 1850, true), [("clanid", "1850".to_string()), ("sessionid", "abc".to_string()), ("follow", "1".to_string())]);
        assert_eq!(follow_fields("abc", 1850, false)[2], ("follow", "0".to_string()));
    }

    #[test]
    fn steams_answer_to_a_follow() {
        assert!(follow_answer(r#"{"success":1}"#).is_ok());
        assert!(follow_answer(r#"{"success":true}"#).is_ok());
        assert!(follow_answer(r#"{"success":{"success":1}}"#).is_ok());
        assert!(follow_answer(r#"{"success":2}"#).is_err());
        assert!(follow_answer("<html>Sign in</html>").is_err());
    }

    #[test]
    fn a_bundle_goes_in_the_cart_by_its_bundle_id() {
        use proto::Message;
        let item = bundle_cart_item(232, false);
        assert_eq!((item.bundleid, item.packageid), (Some(232), None));
        // Field 2 (varint) is the bundle.
        assert_eq!(&item.encode_to_vec()[..3], &[0x10, 0xE8, 0x01]);
    }
}

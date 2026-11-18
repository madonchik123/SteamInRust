//! The Points Shop's catalog (Steam's `ILoyaltyRewardsService`). Browsing it
//! needs no sign-in; the points balance and redeeming do, and aren't here.

use crate::{Result, StoreClient};
use model::AppId;
use serde::Deserialize;

const API: &str = "https://api.steampowered.com/ILoyaltyRewardsService/QueryRewardItems/v1/";
/// Where the items' pictures live, under the game's app id.
const IMAGES: &str = "https://shared.fastly.steamstatic.com/community_assets/images/items";

/// Kinds of items (Steam's community item classes).
pub mod class {
    pub const BACKGROUND: u32 = 3;
    pub const EMOTICON: u32 = 4;
    pub const PROFILE: u32 = 8;
    pub const STICKER: u32 = 11;
    pub const CHAT_EFFECT: u32 = 12;
    pub const MINI_BACKGROUND: u32 = 13;
    pub const AVATAR_FRAME: u32 = 14;
    pub const AVATAR: u32 = 15;
    pub const KEYBOARD: u32 = 16;
    pub const STARTUP_MOVIE: u32 = 17;
}

/// One item you can get with points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RewardItem {
    pub defid: u64,
    /// The game it comes from.
    pub app_id: AppId,
    /// What kind of item ([`class`]).
    pub class: u32,
    pub name: String,
    pub description: String,
    /// Price in points.
    pub cost: u64,
    /// Its picture (empty if Steam has none).
    pub image: String,
    pub animated: bool,
}

/// One page of the catalog.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RewardPage {
    pub items: Vec<RewardItem>,
    /// How many match in all.
    pub total: u64,
    /// For the next page; `None` at the end.
    pub next: Option<String>,
}

#[derive(Deserialize)]
struct Raw {
    response: RawResponse,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawResponse {
    definitions: Vec<RawDef>,
    total_count: u64,
    next_cursor: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawDef {
    appid: AppId,
    defid: u64,
    community_item_class: u32,
    point_cost: String,
    community_item_data: RawData,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawData {
    item_title: String,
    item_name: String,
    item_description: String,
    item_image_large: String,
    item_image_small: String,
    animated: bool,
}

impl StoreClient {
    /// Points Shop items of the given kinds (all kinds when empty), from
    /// the given games (any game when empty), `count` at a time from
    /// `cursor` (`None`: the first page).
    pub async fn reward_items(&self, classes: &[u32], apps: &[AppId], cursor: Option<&str>, count: u32) -> Result<RewardPage> {
        let mut q: Vec<(String, String)> = vec![("count".into(), count.to_string()), ("language".into(), self.language.clone())];
        for (i, c) in classes.iter().enumerate() {
            q.push((format!("community_item_classes[{i}]"), c.to_string()));
        }
        for (i, a) in apps.iter().enumerate() {
            q.push((format!("appids[{i}]"), a.to_string()));
        }
        if let Some(c) = cursor {
            q.push(("cursor".into(), c.to_string()));
        }
        let raw: Raw = self.http.get(API).query(&q).send().await?.error_for_status()?.json().await?;
        Ok(page(raw.response))
    }
}

fn page(r: RawResponse) -> RewardPage {
    let items = r
        .definitions
        .into_iter()
        .map(|d| {
            let data = d.community_item_data;
            let file = if data.item_image_large.is_empty() { data.item_image_small } else { data.item_image_large };
            RewardItem {
                defid: d.defid,
                app_id: d.appid,
                class: d.community_item_class,
                name: if data.item_title.is_empty() { data.item_name } else { data.item_title },
                description: data.item_description,
                cost: d.point_cost.parse().unwrap_or(0),
                image: if file.is_empty() { String::new() } else { format!("{IMAGES}/{}/{file}", d.appid) },
                animated: data.animated,
            }
        })
        .collect();
    RewardPage { items, total: r.total_count, next: r.next_cursor.filter(|c| !c.is_empty()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_page() {
        let json = r#"{"response":{"definitions":[{"appid":2986370,"defid":4118398,"type":1,"community_item_class":3,"point_cost":"500","community_item_data":{"item_name":"Golden Hour","item_title":"Golden Hour","item_description":"Sun down.","item_image_large":"62e7.jpg","animated":false}}],"total_count":158149,"count":1,"next_cursor":"AoJ4"}}"#;
        let raw: Raw = serde_json::from_str(json).unwrap();
        let p = page(raw.response);
        assert_eq!(p.total, 158149);
        assert_eq!(p.next.as_deref(), Some("AoJ4"));
        let i = &p.items[0];
        assert_eq!((i.app_id, i.class, i.cost, i.name.as_str()), (2986370, class::BACKGROUND, 500, "Golden Hour"));
        assert!(i.image.ends_with("/2986370/62e7.jpg"));
    }
}

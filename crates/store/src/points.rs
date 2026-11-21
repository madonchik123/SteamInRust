//! The Points Shop's catalog (Steam's `ILoyaltyRewardsService`). Browsing it
//! needs no sign-in; the balance and redeeming go through the signed-in
//! client (`client::points`).

use crate::{Result, StoreClient};
use model::AppId;
use serde::Deserialize;

const API: &str = "https://api.steampowered.com/ILoyaltyRewardsService/QueryRewardItems/v1/";
/// Where the items' pictures live, under the game's app id.
const IMAGES: &str = "https://shared.fastly.steamstatic.com/community_assets/images/items";

/// Kinds of items (Steam's community item classes).
pub mod class {
    pub const BADGE: u32 = 1;
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
    /// Steam's kind of reward: 1 a single item, 5 and 6 bundles.
    pub reward_type: u32,
    /// The item's type within its game (with the game, what identifies it).
    pub item_type: u32,
    /// Its video (MP4), for items that move.
    pub movie: String,
    /// Its animated picture (GIF), when it has one.
    pub gif: String,
}

impl RewardItem {
    /// A single item with a price, which `RedeemPoints` can get. Bundles
    /// (their price depends on what you own) and seasonal badges (priced by
    /// level) stay on Steam's page.
    pub fn redeemable_here(&self) -> bool {
        self.reward_type == 1 && self.class != class::BADGE && self.cost > 0 && u32::try_from(self.defid).is_ok()
    }

    /// Steam only offers it for games on your eligible list: plain
    /// backgrounds and emoticons (animated backgrounds are open to anyone).
    pub fn needs_eligible_game(&self) -> bool {
        self.class == class::EMOTICON || (self.class == class::BACKGROUND && !self.animated)
    }

    /// How to wear it (the client's profile item kinds): 0 background, 1
    /// mini-profile background, 2 avatar frame, 3 animated avatar.
    pub fn equip_kind(&self) -> Option<u8> {
        match self.class {
            class::BACKGROUND => Some(0),
            class::MINI_BACKGROUND => Some(1),
            class::AVATAR_FRAME => Some(2),
            class::AVATAR => Some(3),
            _ => None,
        }
    }
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
    #[serde(rename = "type")]
    reward_type: u32,
    community_item_class: u32,
    community_item_type: u32,
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
    item_movie_mp4: String,
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
            let url = |file: &str| if file.is_empty() { String::new() } else { format!("{IMAGES}/{}/{file}", d.appid) };
            // AVIF pictures can't be shown here (those items have a video).
            let shown = |file: &String| !file.is_empty() && !file.ends_with(".avif");
            let file = [&data.item_image_large, &data.item_image_small].into_iter().find(|f| shown(f)).cloned().unwrap_or_default();
            let gif = [&data.item_image_small, &data.item_image_large].into_iter().find(|f| f.ends_with(".gif")).map(|f| url(f)).unwrap_or_default();
            RewardItem {
                defid: d.defid,
                app_id: d.appid,
                class: d.community_item_class,
                name: if data.item_title.is_empty() { data.item_name } else { data.item_title },
                description: data.item_description,
                cost: d.point_cost.parse().unwrap_or(0),
                image: url(&file),
                animated: data.animated,
                reward_type: d.reward_type,
                item_type: d.community_item_type,
                movie: url(&data.item_movie_mp4),
                gif,
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

    fn item(reward_type: u32, class: u32, animated: bool) -> RewardItem {
        RewardItem { defid: 7, cost: 500, reward_type, class, animated, ..Default::default() }
    }

    #[test]
    fn reads_what_redeeming_and_previews_need() {
        let json = r#"{"response":{"definitions":[
            {"appid":4197900,"defid":1,"type":1,"community_item_class":15,"community_item_type":3,"point_cost":"3000","community_item_data":{"item_title":"A","item_image_small":"a.gif","item_image_large":"a.jpg","animated":true}},
            {"appid":3095380,"defid":2,"type":1,"community_item_class":17,"community_item_type":100,"point_cost":"3000","community_item_data":{"item_title":"B","item_image_small":"100/s.avif","item_image_large":"100/l.avif","item_movie_mp4":"100/m.mp4"}}
        ]}}"#;
        let p = page(serde_json::from_str::<Raw>(json).unwrap().response);
        let a = &p.items[0];
        assert_eq!((a.reward_type, a.item_type), (1, 3));
        assert!(a.gif.ends_with("/4197900/a.gif") && a.image.ends_with("/a.jpg") && a.movie.is_empty());
        // An AVIF can't be drawn: no picture, but its video plays.
        let b = &p.items[1];
        assert!(b.image.is_empty() && b.movie.ends_with("/3095380/100/m.mp4"));
    }

    #[test]
    fn what_can_be_redeemed_here() {
        assert!(item(1, class::AVATAR, true).redeemable_here());
        // Bundles, seasonal badges, and anything without a readable price stay on Steam.
        assert!(!item(6, 0, false).redeemable_here());
        assert!(!item(5, 0, false).redeemable_here());
        assert!(!item(1, class::BADGE, false).redeemable_here());
        assert!(!RewardItem { cost: 0, ..item(1, class::AVATAR, false) }.redeemable_here());
        assert!(!RewardItem { defid: u64::MAX, ..item(1, class::AVATAR, false) }.redeemable_here());
    }

    #[test]
    fn which_items_need_an_eligible_game() {
        assert!(item(1, class::EMOTICON, false).needs_eligible_game());
        assert!(item(1, class::BACKGROUND, false).needs_eligible_game());
        assert!(!item(1, class::BACKGROUND, true).needs_eligible_game());
        assert!(!item(1, class::AVATAR, true).needs_eligible_game());
        assert!(!item(1, class::STICKER, false).needs_eligible_game());
    }

    #[test]
    fn which_items_can_be_worn() {
        assert_eq!(item(1, class::BACKGROUND, false).equip_kind(), Some(0));
        assert_eq!(item(1, class::MINI_BACKGROUND, false).equip_kind(), Some(1));
        assert_eq!(item(1, class::AVATAR_FRAME, false).equip_kind(), Some(2));
        assert_eq!(item(1, class::AVATAR, false).equip_kind(), Some(3));
        assert_eq!(item(1, class::EMOTICON, false).equip_kind(), None);
    }
}

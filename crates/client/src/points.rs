//! Steam Points: your balance, which Points Shop items you own, and getting
//! one, with the same calls Steam's Points Shop page makes
//! (`LoyaltyRewards.*`, `Quest.GetCommunityInventory`).
//!
//! Spending is never retried here: a request that got no answer may still
//! have gone through.

use model::{AppId, SteamId};
use net::Connection;
use proto::webui::*;

/// A Points Shop item on your account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedReward {
    pub app_id: AppId,
    /// The item's type within its game (with the game, what identifies it).
    pub item_type: u32,
    /// Its community item id (what equipping it takes).
    pub id: u64,
}

/// Why Steam didn't hand the item over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedeemFail {
    NotEnoughPoints,
    /// The cost isn't what was shown any more; nothing was spent.
    PriceChanged,
    /// Steam is already handling the same request.
    Duplicate,
    NoLongerAvailable,
    NotAllowed,
    NoPrivileges,
    /// No answer came: the points may or may not have been spent.
    NoAnswer,
    /// Any other refusal, in Steam's words when it gave any.
    Other(String),
}

impl RedeemFail {
    /// From Steam's result code, as its Points Shop page reads them.
    pub fn from_eresult(code: i32, message: &str) -> Self {
        match code {
            107 => Self::NotEnoughPoints,
            11 => Self::PriceChanged,
            29 => Self::Duplicate,
            26 => Self::NoLongerAvailable,
            15 => Self::NotAllowed,
            24 => Self::NoPrivileges,
            _ if message.trim().is_empty() => Self::Other(format!("error {code}")),
            _ => Self::Other(message.trim().to_string()),
        }
    }

    fn from_net(e: net::Error) -> Self {
        match e {
            net::Error::EResult(code, message) => Self::from_eresult(code, &message),
            // Sent, but no reply: it can't be said that nothing happened.
            net::Error::Timeout | net::Error::Disconnected => Self::NoAnswer,
            other => Self::Other(other.to_string()),
        }
    }
}

/// The request to get item `defid`. `cost` is the price the user agreed
/// to: Steam refuses if the item costs anything else now.
pub fn redeem_request(defid: u32, cost: u64) -> CLoyaltyRewardsRedeemPointsRequest {
    CLoyaltyRewardsRedeemPointsRequest { defid: Some(defid), expected_points_cost: Some(cost as i64) }
}

fn balance_of(r: &CLoyaltyRewardsGetSummaryResponse) -> i64 {
    r.summary.as_ref().map(|s| s.points()).unwrap_or(0)
}

fn owned_of(r: &CQuestGetCommunityInventoryResponse) -> Vec<OwnedReward> {
    r.items.iter().map(|i| OwnedReward { app_id: i.appid(), item_type: i.item_type(), id: i.communityitemid() }).collect()
}

fn eligible_of(r: &CLoyaltyRewardsGetEligibleAppsResponse) -> Vec<AppId> {
    r.apps.iter().map(|a| a.appid()).collect()
}

/// Your Steam Points.
pub async fn balance(conn: &Connection, me: SteamId) -> Result<i64, String> {
    let req = CLoyaltyRewardsGetSummaryRequest { steamid: Some(me) };
    let r: CLoyaltyRewardsGetSummaryResponse = conn.call("LoyaltyRewards.GetSummary#1", &req).await.map_err(|e| e.to_string())?;
    Ok(balance_of(&r))
}

/// The games whose plain backgrounds and emoticons you can get.
pub async fn eligible(conn: &Connection) -> Result<Vec<AppId>, String> {
    let r: CLoyaltyRewardsGetEligibleAppsResponse = conn.call("LoyaltyRewards.GetEligibleApps#1", &CLoyaltyRewardsGetEligibleAppsRequest {}).await.map_err(|e| e.to_string())?;
    Ok(eligible_of(&r))
}

/// The items you own from these games.
pub async fn inventory(conn: &Connection, apps: &[AppId]) -> Result<Vec<OwnedReward>, String> {
    let req = CQuestGetCommunityInventoryRequest { filter_appids: apps.to_vec() };
    let r: CQuestGetCommunityInventoryResponse = conn.call("Quest.GetCommunityInventory#1", &req).await.map_err(|e| e.to_string())?;
    Ok(owned_of(&r))
}

/// Spend points on an item; its community item id on success.
pub async fn redeem(conn: &Connection, defid: u32, cost: u64) -> Result<u64, RedeemFail> {
    let r: CLoyaltyRewardsRedeemPointsResponse = conn.call("LoyaltyRewards.RedeemPoints#1", &redeem_request(defid, cost)).await.map_err(RedeemFail::from_net)?;
    Ok(r.communityitemid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Message;

    #[test]
    fn the_request_carries_the_agreed_price() {
        let req = redeem_request(4118398, 500);
        // Field 1 (varint) the item, field 2 (varint) the cost.
        let mut want = vec![0x08];
        prost::encoding::encode_varint(4118398, &mut want);
        want.push(0x10);
        prost::encoding::encode_varint(500, &mut want);
        assert_eq!(req.encode_to_vec(), want);
    }

    #[test]
    fn reads_the_new_items_id() {
        // communityitemid = 300 (field 1, varint).
        let r = CLoyaltyRewardsRedeemPointsResponse::decode(&[0x08, 0xAC, 0x02][..]).unwrap();
        assert_eq!(r.communityitemid(), 300);
    }

    #[test]
    fn reads_the_balance() {
        let r = CLoyaltyRewardsGetSummaryResponse { summary: Some(CLoyaltyRewardsGetSummaryResponseSummary { points: Some(12_500), points_earned: Some(20_000), points_spent: Some(7_500) }), ..Default::default() };
        let back = CLoyaltyRewardsGetSummaryResponse::decode(r.encode_to_vec().as_slice()).unwrap();
        assert_eq!(balance_of(&back), 12_500);
        assert_eq!(balance_of(&CLoyaltyRewardsGetSummaryResponse::default()), 0);
    }

    #[test]
    fn reads_owned_items_and_eligible_games() {
        let r = CQuestGetCommunityInventoryResponse {
            items: vec![CQuestCommunityItem { communityitemid: Some(77), item_type: Some(17), appid: Some(2986370), ..Default::default() }],
        };
        assert_eq!(owned_of(&r), [OwnedReward { app_id: 2986370, item_type: 17, id: 77 }]);
        let e = CLoyaltyRewardsGetEligibleAppsResponse {
            apps: vec![CLoyaltyRewardsGetEligibleAppsResponseEligibleApp { appid: Some(620), owned: Some(true), ..Default::default() }],
        };
        assert_eq!(eligible_of(&e), [620]);
    }

    #[test]
    fn steams_refusals_are_named() {
        assert_eq!(RedeemFail::from_eresult(107, ""), RedeemFail::NotEnoughPoints);
        assert_eq!(RedeemFail::from_eresult(11, ""), RedeemFail::PriceChanged);
        assert_eq!(RedeemFail::from_eresult(29, ""), RedeemFail::Duplicate);
        assert_eq!(RedeemFail::from_eresult(26, ""), RedeemFail::NoLongerAvailable);
        assert_eq!(RedeemFail::from_eresult(15, ""), RedeemFail::NotAllowed);
        assert_eq!(RedeemFail::from_eresult(24, ""), RedeemFail::NoPrivileges);
        assert_eq!(RedeemFail::from_eresult(2, " Busy "), RedeemFail::Other("Busy".into()));
        assert_eq!(RedeemFail::from_eresult(2, ""), RedeemFail::Other("error 2".into()));
    }

    #[test]
    fn no_answer_is_not_a_refusal() {
        // These must never read as "nothing was spent".
        assert_eq!(RedeemFail::from_net(net::Error::Timeout), RedeemFail::NoAnswer);
        assert_eq!(RedeemFail::from_net(net::Error::Disconnected), RedeemFail::NoAnswer);
        assert_eq!(RedeemFail::from_net(net::Error::EResult(107, String::new())), RedeemFail::NotEnoughPoints);
    }
}

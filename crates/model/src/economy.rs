//! Buying on the Community Market, gems, booster packs and pending gifts:
//! the data, and the sums that decide what you're told you'll pay.

use crate::{AppId, SteamId};

/// A Sack of Gems holds this many gems.
pub const GEMS_PER_SACK: u64 = 1000;

/// Your saved billing address. Steam's purchase forms send it back with each
/// purchase. Never printed, so it can't end up in a log.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct BillingAddress {
    pub first_name: String,
    pub last_name: String,
    pub address: String,
    pub address_two: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
}

impl std::fmt::Debug for BillingAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BillingAddress(hidden)")
    }
}

/// What Steam needs to know before you can buy: your wallet and local tax.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PurchaseInfo {
    /// In cents.
    pub wallet_balance: i64,
    pub wallet_currency: i32,
    /// The wallet currency's smallest market amount, in cents.
    pub market_minimum: u64,
    /// Steam wants a billing address first; that happens on its website.
    pub requires_billing: bool,
    /// Tax on Steam's fee, in percent (0 almost everywhere).
    pub tax_percent: f64,
    pub country: String,
    pub billing: BillingAddress,
}

impl PurchaseInfo {
    /// The lowest price per item Steam accepts for a buy order.
    pub fn min_order_price(&self) -> u64 {
        3 * self.market_minimum.max(1)
    }
}

/// One item for sale on the Market, per single item (a listing can hold a
/// stack, and buying takes one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketListing {
    pub listing_id: u64,
    pub app_id: AppId,
    pub hash_name: String,
    pub name: String,
    pub icon: String,
    pub currency: i32,
    /// What the seller gets, in cents.
    pub price: u64,
    /// Steam's fee and the game's fee together.
    pub fee: u64,
    pub steam_fee: u64,
    pub game_fee: u64,
}

/// The exact sums of a purchase, in cents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListingCost {
    pub subtotal: u64,
    pub fee: u64,
    pub tax: u64,
    pub total: u64,
}

impl MarketListing {
    /// The same sums Steam's buy dialog makes.
    pub fn cost(&self, tax_percent: f64) -> ListingCost {
        if self.fee == 0 {
            return ListingCost { subtotal: self.price, fee: 0, tax: 0, total: self.price };
        }
        let tax = if self.fee > 2 && tax_percent > 0.0 { (self.fee as f64 * tax_percent / 100.0 + 0.5).floor() as u64 } else { 0 };
        ListingCost { subtotal: self.price, fee: self.fee, tax, total: self.price + self.fee + tax }
    }
}

/// Tax on a buy order, the way Steam's order dialog works it out: it splits
/// the fee back out of the total, then taxes the fee.
pub fn order_tax(price: u64, quantity: u64, tax_percent: f64) -> u64 {
    if tax_percent <= 0.0 {
        return 0;
    }
    let total = price * quantity;
    if total < 3 {
        return 0;
    }
    let mut fee = 0;
    if total - 2 <= 20 {
        fee = 2;
    } else {
        let mut base = total * 100 / 115;
        for _ in 0..3 {
            fee = (base * 10 / 100).max(1) + (base * 5 / 100).max(1);
            if base + fee < total {
                base += 1;
            } else {
                break;
            }
        }
    }
    (fee as f64 * tax_percent / 100.0 + 0.5).floor() as u64
}

/// The most a buy order can take from your wallet, in cents.
pub fn order_total(price: u64, quantity: u64, tax_percent: f64) -> u64 {
    price * quantity + order_tax(price, quantity, tax_percent)
}

/// What you can do with a Steam item, from the actions Steam attaches to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    /// Turn it into gems. `game` is the game the item belongs to.
    Grind { game: AppId },
    OpenBooster { game: AppId },
    PackGems { game: AppId },
    UnpackSack { game: AppId },
}

/// Read Steam's item action links (`javascript:OpenBooster( 440, '1' )`).
pub fn item_actions(links: &[(String, String)]) -> Vec<ItemAction> {
    fn args<'a>(link: &'a str, name: &str) -> Option<Vec<&'a str>> {
        let start = link.find(name)? + name.len();
        let rest = link[start..].trim_start().strip_prefix('(')?;
        let inner = &rest[..rest.find(')')?];
        Some(inner.split(',').map(|a| a.trim().trim_matches(['\'', '"'])).collect())
    }
    let game = |a: &[&str], i: usize| a.get(i).and_then(|x| x.parse::<AppId>().ok()).filter(|g| *g != 0);
    let sizes = |a: &[&str]| (a.get(2).and_then(|x| x.parse::<u64>().ok()), a.get(3).and_then(|x| x.parse::<u64>().ok()));
    links
        .iter()
        .filter_map(|(_, link)| {
            if let Some(a) = args(link, "GetGooValue") {
                return Some(ItemAction::Grind { game: game(&a, 2)? });
            }
            if let Some(a) = args(link, "OpenBooster") {
                return Some(ItemAction::OpenBooster { game: game(&a, 0)? });
            }
            if let Some(a) = args(link, "UnpackGameGooFromBarrel") {
                return (sizes(&a) == (Some(GEMS_PER_SACK), Some(1))).then_some(ItemAction::UnpackSack { game: game(&a, 0)? });
            }
            if let Some(a) = args(link, "PackGameGooIntoBarrel") {
                return (sizes(&a) == (Some(1), Some(GEMS_PER_SACK))).then_some(ItemAction::PackGems { game: game(&a, 0)? });
            }
            None
        })
        .collect()
}

/// A game you could make a booster pack for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoosterGame {
    pub app_id: AppId,
    pub name: String,
    pub series: u32,
    /// Cost in gems.
    pub gems: u64,
    pub available: bool,
    /// When it can next be made, as Steam words it.
    pub available_at: Option<String>,
}

/// Steam's booster pack creator: the packs and your gems.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoosterCatalog {
    pub games: Vec<BoosterGame>,
    pub gems: u64,
    pub tradable_gems: u64,
    pub untradable_gems: u64,
}

/// Whether a pack can be made here, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoosterChoice {
    /// Make it, with this value for Steam's `tradability_preference`.
    Make { preference: u8 },
    /// Already made one for this game recently.
    Unavailable,
    NotEnoughGems,
    /// You'd have to spend gems that can't be traded, which Steam asks about
    /// on its own page.
    UseSteam,
}

impl BoosterCatalog {
    pub fn choice(&self, game: &BoosterGame) -> BoosterChoice {
        if !game.available {
            BoosterChoice::Unavailable
        } else if self.gems < game.gems {
            BoosterChoice::NotEnoughGems
        } else if self.untradable_gems == 0 {
            // Steam's default when it has nothing to ask.
            BoosterChoice::Make { preference: 2 }
        } else if self.tradable_gems >= game.gems {
            // Steam's "tradable pack": only tradable gems are spent.
            BoosterChoice::Make { preference: 1 }
        } else {
            BoosterChoice::UseSteam
        }
    }
}

/// A card that came out of a booster pack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoosterCard {
    pub name: String,
    pub image: String,
    pub series: u32,
    pub foil: bool,
}

/// A gift someone sent you that's waiting for your answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingGift {
    pub id: u64,
    /// The game or item. Empty when Steam's page didn't say.
    pub name: String,
    pub sender: SteamId,
    pub sender_name: String,
    pub message: String,
    pub image: String,
    pub can_add_to_library: bool,
    pub can_keep_in_inventory: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_sums_match_steam() {
        let l = MarketListing { price: 53, fee: 7, steam_fee: 2, game_fee: 5, ..Default::default() };
        assert_eq!(l.cost(0.0), ListingCost { subtotal: 53, fee: 7, tax: 0, total: 60 });
        // 7 * 8.25% = 0.58, rounds to 1.
        assert_eq!(l.cost(8.25), ListingCost { subtotal: 53, fee: 7, tax: 1, total: 61 });
        // No tax on a fee of 2 or less, and none when there's no fee.
        let small = MarketListing { price: 3, fee: 2, ..Default::default() };
        assert_eq!(small.cost(20.0).tax, 0);
        let free = MarketListing { price: 40, ..Default::default() };
        assert_eq!(free.cost(20.0), ListingCost { subtotal: 40, fee: 0, tax: 0, total: 40 });
    }

    #[test]
    fn order_sums_match_steam() {
        assert_eq!(order_total(60, 3, 0.0), 180);
        assert_eq!(order_tax(1, 2, 10.0), 0);
        // Up to 22 cents the fee counts as 2.
        assert_eq!(order_tax(11, 2, 50.0), 1);
        // 115 total: base 100, fees 10 + 5 = 15.
        assert_eq!(order_tax(115, 1, 10.0), 2);
        assert_eq!(order_total(115, 1, 10.0), 117);
    }

    #[test]
    fn reads_steams_item_actions() {
        let links: Vec<(String, String)> = [
            "https://steamcommunity.com/my/gamecards/730/",
            "javascript:GetGooValue( '%contextid%', '123', 730, 6, 0 )",
            "javascript:OpenBooster( 465540, '123' )",
            "javascript:UnpackGameGooFromBarrel( 753, '123', 1000, 1, this )",
            "javascript:PackGameGooIntoBarrel( 753, '123', 1, 1000, this )",
            "javascript:PackGameGooIntoBarrel( 753, '123', 1, 7, this )",
        ]
        .iter()
        .map(|l| (String::new(), l.to_string()))
        .collect();
        assert_eq!(
            item_actions(&links),
            vec![ItemAction::Grind { game: 730 }, ItemAction::OpenBooster { game: 465540 }, ItemAction::UnpackSack { game: 753 }, ItemAction::PackGems { game: 753 }]
        );
    }

    #[test]
    fn booster_choice_never_spends_untradable_gems_silently() {
        let game = BoosterGame { gems: 400, available: true, ..Default::default() };
        let cat = |gems, tradable, untradable| BoosterCatalog { games: vec![], gems, tradable_gems: tradable, untradable_gems: untradable };
        assert_eq!(cat(500, 500, 0).choice(&game), BoosterChoice::Make { preference: 2 });
        assert_eq!(cat(900, 500, 400).choice(&game), BoosterChoice::Make { preference: 1 });
        assert_eq!(cat(500, 100, 400).choice(&game), BoosterChoice::UseSteam);
        assert_eq!(cat(300, 300, 0).choice(&game), BoosterChoice::NotEnoughGems);
        let used = BoosterGame { available: false, ..game };
        assert_eq!(cat(900, 900, 0).choice(&used), BoosterChoice::Unavailable);
    }

    #[test]
    fn billing_address_is_never_printed() {
        let b = BillingAddress { first_name: "Private".into(), ..Default::default() };
        assert!(!format!("{b:?}").contains("Private"));
    }
}

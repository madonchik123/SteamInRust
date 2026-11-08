//! "Can I gift this to that friend?" before checkout.
//!
//! Steam refuses a gift when the game is much cheaper in the friend's region
//! than in yours. Valve doesn't document the rule; players consistently report
//! that the gift is blocked when the friend's price is more than about 10%
//! below yours, compared in a common currency (SteamDB's "converted price").
//! We estimate the same way: both regions' base prices (before discounts)
//! converted to US dollars. It's an estimate; Steam decides at checkout.

use crate::{Result, StoreClient};
use model::AppId;
use std::collections::HashMap;

/// The friend's price may be at most this much lower than yours.
pub const LIMIT: f64 = 0.10;

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Their price isn't much lower: Steam should accept the gift.
    Ok,
    /// Their price is more than ~10% lower: Steam will most likely refuse.
    Blocked,
    /// No country on their profile, no price there, or no exchange rate.
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct GiftCheck {
    pub country: Option<String>,
    /// Base prices: (cents, ISO currency).
    pub yours: Option<(u64, String)>,
    pub theirs: Option<(u64, String)>,
    /// Their price compared with yours, in percent (-25.0 = 25% cheaper there).
    pub difference: Option<f64>,
    pub verdict: Verdict,
}

impl StoreClient {
    /// The two-letter country on someone's public profile (its flag), if set.
    pub async fn profile_country(&self, steam_id: u64) -> Result<Option<String>> {
        let html = self.http.get(format!("https://steamcommunity.com/profiles/{steam_id}")).send().await?.error_for_status()?.text().await?;
        Ok(html.split("countryflags/").nth(1).and_then(|rest| rest.split('.').next()).filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic())).map(|c| c.to_lowercase()))
    }

    /// The game's price before discounts in a country: (cents, currency).
    pub async fn base_price(&self, app: AppId, country: &str) -> Result<Option<(u64, String)>> {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={app}&cc={country}&filters=price_overview");
        let v: serde_json::Value = self.http.get(url).send().await?.error_for_status()?.json().await?;
        let p = &v[app.to_string()]["data"]["price_overview"];
        Ok(p["initial"].as_u64().zip(p["currency"].as_str().map(str::to_string)))
    }

    /// Units of each currency per US dollar (free public rates, no key).
    pub async fn usd_rates(&self) -> Result<HashMap<String, f64>> {
        let v: serde_json::Value = self.http.get("https://open.er-api.com/v6/latest/USD").send().await?.error_for_status()?.json().await?;
        Ok(v["rates"].as_object().map(|m| m.iter().filter_map(|(k, r)| r.as_f64().map(|r| (k.clone(), r))).collect()).unwrap_or_default())
    }

    /// Can `app` bought in `my_country` be gifted to `friend`?
    pub async fn gift_check(&self, app: AppId, my_country: &str, friend: u64) -> Result<GiftCheck> {
        let country = self.profile_country(friend).await.unwrap_or(None);
        let mut check = GiftCheck { country: country.clone(), yours: None, theirs: None, difference: None, verdict: Verdict::Unknown(String::new()) };
        let Some(cc) = country else {
            check.verdict = Verdict::Unknown("no country".into());
            return Ok(check);
        };
        check.yours = self.base_price(app, my_country).await?;
        check.theirs = self.base_price(app, &cc).await?;
        let (Some((mine, my_cur)), Some((theirs, their_cur))) = (check.yours.clone(), check.theirs.clone()) else {
            check.verdict = Verdict::Unknown("no price".into());
            return Ok(check);
        };
        let rates = if my_cur == their_cur { HashMap::new() } else { self.usd_rates().await.unwrap_or_default() };
        check.difference = compare(mine, &my_cur, theirs, &their_cur, &rates);
        check.verdict = match check.difference {
            Some(d) => verdict(d),
            None => Verdict::Unknown("no rate".into()),
        };
        Ok(check)
    }
}

/// Their price vs yours in percent, both converted to dollars.
pub fn compare(mine: u64, my_cur: &str, theirs: u64, their_cur: &str, usd_rates: &HashMap<String, f64>) -> Option<f64> {
    let usd = |cents: u64, cur: &str| -> Option<f64> {
        if cur == "USD" {
            return Some(cents as f64);
        }
        usd_rates.get(cur).filter(|r| **r > 0.0).map(|r| cents as f64 / r)
    };
    let (a, b) = if my_cur == their_cur { (mine as f64, theirs as f64) } else { (usd(mine, my_cur)?, usd(theirs, their_cur)?) };
    (a > 0.0).then(|| (b - a) / a * 100.0)
}

pub fn verdict(difference_percent: f64) -> Verdict {
    if difference_percent < -LIMIT * 100.0 { Verdict::Blocked } else { Verdict::Ok }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_percent_rule() {
        let rates: HashMap<String, f64> = [("KZT".to_string(), 500.0), ("TRY".to_string(), 40.0)].into();
        // 5 000₸ = $10 vs $10 in the US: same price, fine.
        assert_eq!(compare(500_000, "KZT", 1000, "USD", &rates).map(verdict), Some(Verdict::Ok));
        // Their $6 vs your $10: 40% cheaper there, blocked.
        let d = compare(1000, "USD", 600, "USD", &rates).unwrap();
        assert!((d + 40.0).abs() < 0.01);
        assert_eq!(verdict(d), Verdict::Blocked);
        // 9% cheaper is still allowed; more expensive always is.
        assert_eq!(verdict(-9.0), Verdict::Ok);
        assert_eq!(verdict(25.0), Verdict::Ok);
        // No rate for a currency: can't tell.
        assert_eq!(compare(1000, "USD", 100, "XYZ", &rates), None);
    }
}

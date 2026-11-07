//! Price history from IsThereAnyDeal's public API (https://docs.isthereanydeal.com).
//! Each user supplies their own free API key. Data is shown unmodified and
//! credited to IsThereAnyDeal, as its terms ask.

use crate::{Error, Result};
use model::AppId;
use serde::Deserialize;

const API: &str = "https://api.isthereanydeal.com";
/// IsThereAnyDeal's shop id for Steam.
const STEAM_SHOP: u32 = 61;

#[derive(Debug, Clone)]
pub struct PricePoint {
    /// Unix seconds.
    pub time: u64,
    pub price: f64,
    pub regular: f64,
    pub cut: u32,
}

#[derive(Debug, Clone)]
pub struct PriceHistory {
    pub points: Vec<PricePoint>,
    pub currency: String,
    /// All-time low on Steam: (price, unix time).
    pub low: Option<(f64, u64)>,
    pub url: Option<String>,
}

pub async fn history(http: &reqwest::Client, key: &str, app: AppId, country: &str) -> Result<PriceHistory> {
    let lookup: Lookup = http
        .get(format!("{API}/games/lookup/v1"))
        .query(&[("key", key), ("appid", &app.to_string())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let game = lookup.game.filter(|_| lookup.found).ok_or(Error::NotFound(app))?;
    let since = "2015-01-01T00:00:00Z";
    let country = country.to_uppercase();
    let raw: Vec<RawLog> = http
        .get(format!("{API}/games/history/v2"))
        .query(&[("key", key), ("id", &game.id), ("country", &country), ("shops", &STEAM_SHOP.to_string()), ("since", since)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let lows: Vec<RawLow> = http
        .post(format!("{API}/games/storelow/v2"))
        .query(&[("key", key), ("country", &country), ("shops", &STEAM_SHOP.to_string())])
        .json(&[&game.id])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut currency = String::new();
    let mut points: Vec<PricePoint> = raw
        .into_iter()
        .filter_map(|r| {
            let deal = r.deal?;
            currency = deal.price.currency.clone();
            Some(PricePoint { time: parse_time(&r.timestamp)?, price: deal.price.amount, regular: deal.regular.amount, cut: deal.cut })
        })
        .collect();
    points.sort_by_key(|p| p.time);
    let low = lows.into_iter().flat_map(|l| l.lows).find(|l| l.shop.id == STEAM_SHOP).map(|l| (l.price.amount, parse_time(&l.timestamp).unwrap_or(0)));
    Ok(PriceHistory { points, currency, low, url: game.urls.map(|u| u.game) })
}

/// "2022-12-27T11:21:08+01:00" -> unix seconds (UTC).
fn parse_time(s: &str) -> Option<u64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = &rest[..8.min(rest.len())];
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    let tz = &rest[8.min(rest.len())..];
    let offset = if tz.len() >= 6 {
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        sign * (tz[1..3].parse::<i64>().ok()? * 3600 + tz[4..6].parse::<i64>().ok()? * 60)
    } else {
        0
    };
    // Days from civil (Howard Hinnant).
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + hh * 3600 + mm * 60 + ss - offset).max(0) as u64)
}

#[derive(Deserialize)]
struct Lookup {
    #[serde(default)]
    found: bool,
    game: Option<Game>,
}
#[derive(Deserialize)]
struct Game {
    id: String,
    urls: Option<Urls>,
}
#[derive(Deserialize)]
struct Urls {
    game: String,
}
#[derive(Deserialize)]
struct RawLog {
    timestamp: String,
    deal: Option<RawDeal>,
}
#[derive(Deserialize)]
struct RawDeal {
    price: Money,
    regular: Money,
    #[serde(default)]
    cut: u32,
}
#[derive(Deserialize)]
struct Money {
    amount: f64,
    #[serde(default)]
    currency: String,
}
#[derive(Deserialize)]
struct RawLow {
    #[serde(default)]
    lows: Vec<RawShopLow>,
}
#[derive(Deserialize)]
struct RawShopLow {
    shop: Shop,
    price: Money,
    timestamp: String,
}
#[derive(Deserialize)]
struct Shop {
    id: u32,
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_iso_times() {
        assert_eq!(super::parse_time("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(super::parse_time("2022-12-27T11:21:08+01:00"), Some(1_672_136_468));
    }
}

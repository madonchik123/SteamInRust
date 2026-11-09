//! Store browsing with filters, via Steam's `IStoreQueryService` (the same
//! query engine the Steam store's search and category pages use).

use crate::{Price, Result, StoreClient};
use model::AppId;
use serde::Deserialize;

const ASSETS: &str = "https://shared.akamai.steamstatic.com/store_item_assets/";

/// Sort orders. Numbers are Steam's (found by probing the API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Popular,
    TopSellers,
    MostPlayed,
    NewAndTrending,
    TopRated,
    Name,
    Oldest,
}

impl Sort {
    pub const ALL: [Sort; 7] = [Sort::Popular, Sort::TopSellers, Sort::MostPlayed, Sort::NewAndTrending, Sort::TopRated, Sort::Name, Sort::Oldest];
    fn id(self) -> i32 {
        match self {
            Sort::Name => 1,
            Sort::Oldest => 2,
            Sort::TopSellers => 10,
            Sort::Popular => 11,
            Sort::MostPlayed => 13,
            Sort::NewAndTrending => 20,
            Sort::TopRated => 21,
        }
    }
    /// English label (translate with i18n).
    pub fn label(self) -> &'static str {
        match self {
            Sort::Popular => "Popular",
            Sort::TopSellers => "Top sellers",
            Sort::MostPlayed => "Most played",
            Sort::NewAndTrending => "New & trending",
            Sort::TopRated => "Top rated",
            Sort::Name => "Name",
            Sort::Oldest => "Oldest first",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filters {
    pub sort: Sort,
    pub tags: Vec<u32>,
    pub exclude_tags: Vec<u32>,
    /// In cents, in the store's currency.
    pub max_price: Option<u32>,
    pub only_discounted: bool,
    pub only_free: bool,
    pub games: bool,
    pub dlc: bool,
    pub software: bool,
    pub demos: bool,
    pub soundtracks: bool,
    pub coming_soon: bool,
    /// Minimum review score (1-9; 7 = "Mostly positive" and up).
    pub min_review_score: Option<u32>,
    pub hidden_descriptors: Vec<u32>,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            sort: Sort::Popular,
            tags: Vec::new(),
            exclude_tags: Vec::new(),
            max_price: None,
            only_discounted: false,
            only_free: false,
            games: true,
            dlc: false,
            software: false,
            demos: false,
            soundtracks: false,
            coming_soon: false,
            min_review_score: None,
            hidden_descriptors: vec![3, 4],
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryItem {
    pub app_id: AppId,
    pub name: String,
    pub header: String,
    pub capsule: Option<String>,
    pub price: Price,
    pub formatted_price: Option<String>,
    /// The price before the discount, as Steam formats it (sales only).
    pub formatted_original: Option<String>,
    pub package_id: Option<u32>,
    pub giftable: bool,
    pub review_label: Option<String>,
    pub review_percent: u32,
    pub review_score: u32,
    pub platforms: (bool, bool, bool),
}

impl StoreClient {
    /// One page of results and the total number of matches.
    pub async fn query(&self, f: &Filters, start: u32, count: u32) -> Result<(Vec<QueryItem>, u64)> {
        let mut filters = serde_json::json!({
            "released_only": !f.coming_soon,
            "coming_soon_only": f.coming_soon,
            "type_filters": {
                "include_games": f.games,
                "include_dlc": f.dlc,
                "include_software": f.software,
                "include_demos": f.demos,
                "include_soundtracks": f.soundtracks,
            },
            "content_descriptors_excluded": f.hidden_descriptors,
        });
        if !f.tags.is_empty() {
            filters["tagids_must_match"] = f.tags.iter().map(|t| serde_json::json!({ "tagids": [t] })).collect();
        }
        if !f.exclude_tags.is_empty() {
            filters["tagids_exclude"] = serde_json::json!(f.exclude_tags);
        }
        let mut price = serde_json::Map::new();
        if let Some(max) = f.max_price {
            price.insert("max_price_cents".into(), max.into());
        }
        if f.only_discounted {
            price.insert("only_discounted".into(), true.into());
        }
        if f.only_free {
            price.insert("only_free_items".into(), true.into());
        }
        if !price.is_empty() {
            filters["price_filters"] = price.into();
        }
        if let Some(min) = f.min_review_score {
            filters["review_score_filter"] = serde_json::json!({ "review_score_min": min });
        }
        let input = serde_json::json!({
            "query_name": "SteamInRust",
            "query": { "start": start, "count": count, "sort": f.sort.id(), "filters": filters },
            "context": { "language": self.language, "country_code": self.country.to_uppercase() },
            "data_request": { "include_basic_info": true, "include_assets": true, "include_reviews": true, "include_platforms": true }
        });
        let resp: Envelope = self
            .http
            .get("https://api.steampowered.com/IStoreQueryService/Query/v1/")
            .query(&[("input_json", input.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let total = resp.response.metadata.map(|m| m.total_matching_records).unwrap_or(0);
        Ok((resp.response.store_items.into_iter().filter(|i| i.appid != 0).map(QueryItem::from).collect(), total))
    }
}

impl StoreClient {
    /// Cards (price, art, reviews) for specific apps, e.g. a wishlist.
    pub async fn items(&self, ids: &[AppId]) -> Result<Vec<QueryItem>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(100) {
            let input = serde_json::json!({
                "ids": chunk.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
                "context": { "language": self.language, "country_code": self.country.to_uppercase() },
                "data_request": { "include_basic_info": true, "include_assets": true, "include_reviews": true, "include_platforms": true }
            });
            let resp: GetItems = self
                .http
                .get("https://api.steampowered.com/IStoreBrowseService/GetItems/v1/")
                .query(&[("input_json", input.to_string())])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            out.extend(resp.response.store_items.into_iter().filter(|i| i.appid != 0).map(QueryItem::from));
        }
        Ok(out)
    }

    /// Names and art for packages (cart lines are packages, not apps).
    /// (package id, name, header art, first app in it) for each package.
    pub async fn packages(&self, ids: &[u32]) -> Result<Vec<(u32, String, String, AppId)>> {
        let input = serde_json::json!({
            "ids": ids.iter().map(|id| serde_json::json!({ "packageid": id })).collect::<Vec<_>>(),
            "context": { "language": self.language, "country_code": self.country.to_uppercase() },
            "data_request": { "include_assets": true }
        });
        let resp: GetPackages = self
            .http
            .get("https://api.steampowered.com/IStoreBrowseService/GetItems/v1/")
            .query(&[("input_json", input.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .response
            .store_items
            .into_iter()
            .map(|p| {
                let art = p
                    .assets
                    .as_ref()
                    .and_then(|a| Some(format!("{ASSETS}{}", a.asset_url_format.as_deref()?.replace("${FILENAME}", a.header.as_deref()?))))
                    .unwrap_or_else(|| model::art::header(p.included_appids.first().copied().unwrap_or(0)));
                (if p.id != 0 { p.id } else { p.packageid }, p.name, art, p.included_appids.first().copied().unwrap_or(0))
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct GetItems {
    response: RawResponse,
}

#[derive(Deserialize)]
struct GetPackages {
    response: RawPackages,
}

#[derive(Deserialize)]
struct RawPackages {
    #[serde(default)]
    store_items: Vec<RawPackage>,
}

#[derive(Deserialize)]
struct RawPackage {
    /// Steam answers with the package id in `id` (older answers: `packageid`).
    #[serde(default)]
    id: u32,
    #[serde(default)]
    packageid: u32,
    #[serde(default)]
    name: String,
    assets: Option<RawAssets>,
    #[serde(default)]
    included_appids: Vec<AppId>,
}

impl From<RawItem> for QueryItem {
    fn from(i: RawItem) -> Self {
        let asset = |file: &Option<String>| -> Option<String> {
            let fmt = i.assets.as_ref()?.asset_url_format.as_deref()?;
            Some(format!("{ASSETS}{}", fmt.replace("${FILENAME}", file.as_deref()?)))
        };
        let assets = i.assets.as_ref();
        let header = assets.and_then(|a| asset(&a.header)).unwrap_or_else(|| model::art::header(i.appid));
        let capsule = assets.and_then(|a| asset(&a.main_capsule));
        let bpo = i.best_purchase_option;
        let parse = |s: &Option<String>| s.as_deref().and_then(|v| v.parse::<u64>().ok());
        let (final_, initial, discount) = match &bpo {
            Some(b) => (parse(&b.final_price_in_cents), parse(&b.original_price_in_cents), b.discount_pct.unwrap_or(0)),
            None => (Some(0), None, 0),
        };
        let reviews = i.reviews.and_then(|r| r.summary_filtered);
        QueryItem {
            app_id: i.appid,
            name: i.name,
            header,
            capsule,
            price: Price { initial: initial.or(final_), final_, discount, currency: String::new() },
            formatted_price: bpo.as_ref().and_then(|b| b.formatted_final_price.clone()),
            formatted_original: bpo.as_ref().filter(|b| b.discount_pct.unwrap_or(0) > 0).and_then(|b| b.formatted_original_price.clone()),
            package_id: bpo.as_ref().and_then(|b| b.packageid),
            giftable: bpo.as_ref().and_then(|b| b.user_can_purchase_as_gift).unwrap_or(false),
            review_label: reviews.as_ref().map(|r| r.review_score_label.clone()).filter(|l| !l.is_empty()),
            review_percent: reviews.as_ref().map(|r| r.percent_positive).unwrap_or(0),
            review_score: reviews.as_ref().map(|r| r.review_score).unwrap_or(0),
            platforms: i.platforms.map(|p| (p.windows, p.mac, p.steamos_linux)).unwrap_or((true, false, false)),
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    response: RawResponse,
}

#[derive(Deserialize)]
struct RawResponse {
    metadata: Option<RawMeta>,
    #[serde(default)]
    store_items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawMeta {
    #[serde(default)]
    total_matching_records: u64,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    appid: AppId,
    #[serde(default)]
    name: String,
    assets: Option<RawAssets>,
    best_purchase_option: Option<RawPurchase>,
    reviews: Option<RawReviews>,
    platforms: Option<RawPlatforms>,
}

#[derive(Deserialize)]
struct RawAssets {
    asset_url_format: Option<String>,
    header: Option<String>,
    main_capsule: Option<String>,
}

#[derive(Deserialize)]
struct RawPurchase {
    packageid: Option<u32>,
    final_price_in_cents: Option<String>,
    original_price_in_cents: Option<String>,
    discount_pct: Option<u8>,
    formatted_final_price: Option<String>,
    formatted_original_price: Option<String>,
    user_can_purchase_as_gift: Option<bool>,
}

#[derive(Deserialize)]
struct RawReviews {
    summary_filtered: Option<RawSummary>,
}

#[derive(Deserialize)]
struct RawSummary {
    #[serde(default)]
    percent_positive: u32,
    #[serde(default)]
    review_score: u32,
    #[serde(default)]
    review_score_label: String,
}

#[derive(Deserialize)]
struct RawPlatforms {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    mac: bool,
    #[serde(default)]
    steamos_linux: bool,
}

#[cfg(test)]
mod package_tests {
    use super::*;

    #[test]
    fn reads_package_ids_from_id() {
        // Trimmed from a real IStoreBrowseService/GetItems answer.
        let json = r#"{"response":{"store_items":[{"item_type":1,"id":244390,"success":1,"name":"Rust","appid":252490,"included_appids":[252490],"best_purchase_option":{"packageid":244390}}]}}"#;
        let r: GetPackages = serde_json::from_str(json).unwrap();
        let p = &r.response.store_items[0];
        assert_eq!((p.id, p.name.as_str(), p.included_appids.clone()), (244390, "Rust", vec![252490]));
    }
}

//! The Steam store, natively: JSON endpoints in, Rust structs out. The UI
//! renders everything itself; no embedded browser.
//!
//! Checkout is the one exception: Steam only takes payments on its website,
//! so "Purchase" opens the store page in the system browser.

pub mod browse;
pub mod events;
pub mod itad;
pub mod query;
pub mod html;

use model::AppId;
use serde::Deserialize;
use std::collections::HashMap;

const BASE: &str = "https://store.steampowered.com/api";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("app {0} isn't available in the store")]
    NotFound(AppId),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Price {
    /// Minor units (cents). `None` = free or unknown.
    pub initial: Option<u64>,
    pub final_: Option<u64>,
    pub discount: u8,
    pub currency: String,
}

impl Price {
    pub fn format(cents: u64, currency: &str) -> String {
        let v = cents as f64 / 100.0;
        match currency {
            "USD" | "" => format!("${v:.2}"),
            "EUR" => format!("{v:.2}€"),
            "GBP" => format!("£{v:.2}"),
            "RUB" => format!("{v:.0} ₽"),
            other => format!("{v:.2} {other}"),
        }
    }
    pub fn label(&self) -> String {
        match self.final_ {
            Some(0) | None => "Free".into(),
            Some(c) => Self::format(c, &self.currency),
        }
    }
    pub fn original_label(&self) -> Option<String> {
        (self.discount > 0).then(|| self.initial.map(|c| Self::format(c, &self.currency))).flatten()
    }
}

/// A store listing card.
#[derive(Debug, Clone)]
pub struct Item {
    pub app_id: AppId,
    pub name: String,
    /// 460x215 header.
    pub header: String,
    /// 616x353 capsule, when the endpoint provides one.
    pub large: Option<String>,
    pub price: Price,
    pub windows: bool,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default)]
pub struct Details {
    pub app_id: AppId,
    pub name: String,
    pub kind: String,
    pub short_description: String,
    pub about: Vec<html::Block>,
    pub header: String,
    pub background: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub categories: Vec<String>,
    pub release_date: String,
    pub coming_soon: bool,
    pub is_free: bool,
    pub price: Option<Price>,
    pub price_text: Option<String>,
    pub screenshots: Vec<Screenshot>,
    pub requirements_min: Vec<html::Block>,
    pub requirements_rec: Vec<html::Block>,
    pub metacritic: Option<u32>,
    pub recommendations: Option<u64>,
    pub platforms: (bool, bool, bool),
    pub languages: String,
    pub website: Option<String>,
    /// Steam content descriptor ids (1 = some nudity, 2 = violence/gore,
    /// 3 = adult-only sexual content, 4 = frequent nudity, 5 = mature).
    pub content_descriptors: Vec<u32>,
    pub content_notes: Option<String>,
    /// Package to add to the cart (the default purchase option).
    pub package_id: Option<u32>,
    /// DLC app ids.
    pub dlc: Vec<AppId>,
}

#[derive(Debug, Clone)]
pub struct Screenshot {
    pub thumb: String,
    pub full: String,
}

#[derive(Clone)]
pub struct StoreClient {
    pub(crate) http: reqwest::Client,
    pub country: String,
    pub language: String,
}

impl Default for StoreClient {
    fn default() -> Self {
        Self::new("us", "english")
    }
}

impl StoreClient {
    pub fn new(country: &str, language: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client");
        Self { http, country: country.into(), language: language.into() }
    }

    /// Price history from IsThereAnyDeal (needs the user's API key).
    pub async fn price_history(&self, key: &str, app: AppId) -> Result<itad::PriceHistory> {
        itad::history(&self.http, key, app, &self.country).await
    }

    /// Same client with a different region/language (prices and text follow them).
    pub fn with_locale(&self, country: &str, language: &str) -> Self {
        Self { http: self.http.clone(), country: country.to_lowercase(), language: language.into() }
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        Ok(self.http.get(url).send().await?.error_for_status()?.json().await?)
    }

    /// Store front page: spotlight items plus the standard rows.
    pub async fn front(&self) -> Result<(Vec<Item>, Vec<Section>)> {
        let q = format!("cc={}&l={}", self.country, self.language);
        let featured: RawFeatured = self.get(&format!("{BASE}/featured?{q}")).await?;
        let mut spotlight: Vec<Item> = featured
            .large_capsules
            .into_iter()
            .chain(featured.featured_win)
            .filter(|i| i.kind == 0)
            .map(Item::from)
            .collect();
        dedup(&mut spotlight);

        let cats: HashMap<String, serde_json::Value> = self.get(&format!("{BASE}/featuredcategories?{q}")).await?;
        let mut sections = Vec::new();
        for key in ["specials", "top_sellers", "new_releases", "coming_soon"] {
            let Some(v) = cats.get(key) else { continue };
            let Ok(cat) = serde_json::from_value::<RawCategory>(v.clone()) else { continue };
            let mut items: Vec<Item> = cat.items.into_iter().filter(|i| i.kind == 0).map(Item::from).collect();
            dedup(&mut items);
            if !items.is_empty() {
                sections.push(Section { id: key.into(), title: cat.name, items });
            }
        }
        Ok((spotlight, sections))
    }

    /// Everything by one developer or publisher (`kind` is "developer" or
    /// "publisher"), like the store's developer and publisher pages.
    pub async fn creator(&self, kind: &str, name: &str) -> Result<Vec<Item>> {
        let mut ids: Vec<AppId> = Vec::new();
        // Steam's search pages give 50 at a time; a few pages covers almost anyone.
        for page in 0..4u32 {
            let resp: serde_json::Value = self
                .http
                .get("https://store.steampowered.com/search/results/")
                .query(&[
                    (kind, name),
                    ("json", "1"),
                    ("infinite", "1"),
                    ("start", &(page * 50).to_string()),
                    ("count", "50"),
                    ("cc", &self.country),
                    ("l", &self.language),
                ])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let html = resp.get("results_html").and_then(|h| h.as_str()).unwrap_or_default();
            let before = ids.len();
            for part in html.split("data-ds-appid=\"").skip(1) {
                // Bundles list several ids ("10,20"); the first is the main app.
                let first = part.split(['"', ',']).next().unwrap_or_default();
                if let Ok(id) = first.parse::<AppId>() {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            let total = resp.get("total_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if ids.len() == before || ids.len() >= total {
                break;
            }
        }
        let cards = self.items(&ids).await?;
        // Keep Steam's order (by relevance / release).
        Ok(ids
            .iter()
            .filter_map(|id| cards.iter().find(|c| c.app_id == *id))
            .map(|c| Item { app_id: c.app_id, name: c.name.clone(), header: c.header.clone(), large: None, price: c.price.clone(), windows: c.platforms.0 })
            .collect())
    }

    pub async fn search(&self, term: &str) -> Result<Vec<Item>> {
        let resp: RawSearch = self
            .http
            .get(format!("{BASE}/storesearch/"))
            .query(&[("term", term), ("cc", &self.country), ("l", &self.language)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .items
            .into_iter()
            .filter(|i| i.kind == "app")
            .map(|i| Item {
                app_id: i.id,
                header: model::art::header(i.id),
                large: None,
                name: i.name,
                windows: i.platforms.windows,
                price: i
                    .price
                    .map(|p| Price {
                        discount: discount(p.initial, p.final_),
                        initial: Some(p.initial),
                        final_: Some(p.final_),
                        currency: p.currency,
                    })
                    .unwrap_or_default(),
            })
            .collect())
    }

    pub async fn details(&self, id: AppId) -> Result<Details> {
        let url = format!("{BASE}/appdetails?appids={id}&cc={}&l={}", self.country, self.language);
        let mut raw: HashMap<String, RawEnvelope> = self.get(&url).await?;
        let d = raw.remove(&id.to_string()).filter(|e| e.success).and_then(|e| e.data).ok_or(Error::NotFound(id))?;
        let (min, rec) = match d.pc_requirements {
            serde_json::Value::Object(m) => (
                m.get("minimum").and_then(|v| v.as_str()).map(requirements).unwrap_or_default(),
                m.get("recommended").and_then(|v| v.as_str()).map(requirements).unwrap_or_default(),
            ),
            _ => Default::default(),
        };
        Ok(Details {
            app_id: id,
            name: d.name,
            kind: d.kind,
            short_description: html::text(&d.short_description),
            about: html::parse(&d.about_the_game),
            header: d.header_image,
            background: d.background_raw.or(d.background).unwrap_or_default(),
            developers: d.developers,
            publishers: d.publishers,
            genres: d.genres.into_iter().map(|g| g.description).collect(),
            categories: d.categories.into_iter().map(|g| g.description).collect(),
            release_date: d.release_date.as_ref().map(|r| r.date.clone()).unwrap_or_default(),
            coming_soon: d.release_date.map(|r| r.coming_soon).unwrap_or(false),
            is_free: d.is_free,
            price_text: d.price_overview.as_ref().map(|p| p.final_formatted.clone()),
            price: d.price_overview.map(|p| Price {
                initial: Some(p.initial),
                final_: Some(p.final_),
                discount: p.discount_percent,
                currency: p.currency,
            }),
            screenshots: d.screenshots.into_iter().map(|s| Screenshot { thumb: s.path_thumbnail, full: s.path_full }).collect(),
            requirements_min: min,
            requirements_rec: rec,
            metacritic: d.metacritic.map(|m| m.score),
            recommendations: d.recommendations.map(|r| r.total),
            platforms: d.platforms.map(|p| (p.windows, p.mac, p.linux)).unwrap_or((true, false, false)),
            languages: html::text(&d.supported_languages),
            website: d.website.filter(|w| !w.is_empty()),
            content_descriptors: d.content_descriptors.as_ref().map(|c| c.ids.clone()).unwrap_or_default(),
            content_notes: d.content_descriptors.and_then(|c| c.notes).filter(|n| !n.is_empty()),
            package_id: d
                .package_groups
                .iter()
                .find(|g| g.name == "default")
                .or(d.package_groups.first())
                .and_then(|g| g.subs.first())
                .map(|s| s.packageid)
                .or(d.packages.first().copied()),
            dlc: d.dlc,
        })
    }
}

/// System requirements without the leading "Minimum:" / "Recommended:" label
/// (the UI shows its own heading).
fn requirements(src: &str) -> Vec<html::Block> {
    let mut blocks = html::parse(src);
    if let Some(html::Block::Paragraph(runs)) = blocks.first() {
        if runs.len() == 1 && runs[0].0.trim_end().ends_with(':') && runs[0].0.len() < 40 {
            blocks.remove(0);
        }
    }
    blocks
}

fn dedup(items: &mut Vec<Item>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.app_id));
}

fn discount(initial: u64, final_: u64) -> u8 {
    if initial == 0 || final_ >= initial { 0 } else { (100 - final_ * 100 / initial) as u8 }
}

/// Store page URL, used for checkout in the browser.
pub fn store_url(id: AppId) -> String {
    format!("https://store.steampowered.com/app/{id}")
}

// ---- raw JSON ----

#[derive(Deserialize)]
struct RawFeatured {
    #[serde(default)]
    large_capsules: Vec<RawItem>,
    #[serde(default)]
    featured_win: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(default)]
    name: String,
    #[serde(default)]
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    id: AppId,
    #[serde(rename = "type", default)]
    kind: u8,
    name: String,
    #[serde(default)]
    discount_percent: u8,
    original_price: Option<u64>,
    final_price: Option<u64>,
    #[serde(default)]
    currency: String,
    #[serde(default)]
    header_image: String,
    large_capsule_image: Option<String>,
    #[serde(default)]
    windows_available: bool,
}

impl From<RawItem> for Item {
    fn from(i: RawItem) -> Self {
        Item {
            app_id: i.id,
            header: if i.header_image.is_empty() { model::art::header(i.id) } else { i.header_image },
            large: i.large_capsule_image.filter(|s| !s.is_empty()),
            name: i.name,
            windows: i.windows_available,
            price: Price {
                initial: i.original_price,
                final_: i.final_price,
                discount: i.discount_percent,
                currency: i.currency,
            },
        }
    }
}

#[derive(Deserialize)]
struct RawSearch {
    #[serde(default)]
    items: Vec<RawSearchItem>,
}

#[derive(Deserialize)]
struct RawSearchItem {
    #[serde(rename = "type")]
    kind: String,
    id: AppId,
    name: String,
    price: Option<RawSearchPrice>,
    #[serde(default)]
    platforms: RawPlatforms,
}

#[derive(Deserialize)]
struct RawSearchPrice {
    currency: String,
    initial: u64,
    #[serde(rename = "final")]
    final_: u64,
}

#[derive(Deserialize, Default)]
struct RawPlatforms {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    mac: bool,
    #[serde(default)]
    linux: bool,
}

#[derive(Deserialize)]
struct RawEnvelope {
    success: bool,
    data: Option<RawDetails>,
}

#[derive(Deserialize)]
struct RawDetails {
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    about_the_game: String,
    #[serde(default)]
    header_image: String,
    background: Option<String>,
    background_raw: Option<String>,
    #[serde(default)]
    is_free: bool,
    #[serde(default)]
    developers: Vec<String>,
    #[serde(default)]
    publishers: Vec<String>,
    #[serde(default)]
    genres: Vec<RawDesc>,
    #[serde(default)]
    categories: Vec<RawDesc>,
    release_date: Option<RawRelease>,
    price_overview: Option<RawPrice>,
    #[serde(default)]
    screenshots: Vec<RawShot>,
    #[serde(default)]
    pc_requirements: serde_json::Value,
    metacritic: Option<RawMeta>,
    recommendations: Option<RawRecs>,
    platforms: Option<RawPlatforms>,
    #[serde(default)]
    supported_languages: String,
    website: Option<String>,
    content_descriptors: Option<RawDescriptors>,
    #[serde(default)]
    packages: Vec<u32>,
    #[serde(default)]
    package_groups: Vec<RawPackageGroup>,
    #[serde(default)]
    dlc: Vec<AppId>,
}

#[derive(Deserialize)]
struct RawPackageGroup {
    #[serde(default)]
    name: String,
    #[serde(default)]
    subs: Vec<RawSub>,
}

#[derive(Deserialize)]
struct RawSub {
    packageid: u32,
}

#[derive(Deserialize)]
struct RawDescriptors {
    #[serde(default)]
    ids: Vec<u32>,
    notes: Option<String>,
}

#[derive(Deserialize)]
struct RawDesc {
    description: String,
}
#[derive(Deserialize)]
struct RawRelease {
    #[serde(default)]
    coming_soon: bool,
    #[serde(default)]
    date: String,
}
#[derive(Deserialize)]
struct RawPrice {
    #[serde(default)]
    currency: String,
    initial: u64,
    #[serde(rename = "final")]
    final_: u64,
    #[serde(default)]
    discount_percent: u8,
    #[serde(default)]
    final_formatted: String,
}
#[derive(Deserialize)]
struct RawShot {
    path_thumbnail: String,
    path_full: String,
}
#[derive(Deserialize)]
struct RawMeta {
    score: u32,
}
#[derive(Deserialize)]
struct RawRecs {
    total: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn discount_math() {
        assert_eq!(super::discount(2000, 1000), 50);
        assert_eq!(super::discount(0, 0), 0);
    }
}

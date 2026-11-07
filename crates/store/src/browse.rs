//! Richer store data from Steam's public `IStoreBrowseService` (the same
//! source Steam's own client uses): review scores, tags and trailers.
//! Also game news from `ISteamNews`.

use crate::{Result, StoreClient};
use model::AppId;
use serde::Deserialize;
use std::collections::HashMap;

const API: &str = "https://api.steampowered.com";
const VIDEO: &str = "https://video.akamai.steamstatic.com/store_trailers";
const IMAGES: &str = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps";

#[derive(Debug, Clone, Default)]
pub struct Reviews {
    /// "Very Positive", "Mixed", ...
    pub label: String,
    pub percent_positive: u32,
    pub count: u64,
    /// 1 (overwhelmingly negative) to 9 (overwhelmingly positive).
    pub score: u32,
}

#[derive(Debug, Clone)]
pub struct Trailer {
    pub name: String,
    pub thumbnail: String,
    /// Short silent loop (MP4).
    pub micro_mp4: Option<String>,
    /// Full trailer as an HLS playlist (H.264).
    pub hls: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowseInfo {
    pub app_id: AppId,
    pub reviews: Option<Reviews>,
    pub tags: Vec<u32>,
    pub trailers: Vec<Trailer>,
    pub content_descriptors: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub author: String,
    /// Plain text excerpt.
    pub excerpt: String,
    /// Unix seconds.
    pub date: u64,
    pub feed: String,
}

impl StoreClient {
    /// Review summary, tags and trailers for a batch of apps.
    pub async fn browse(&self, ids: &[AppId]) -> Result<Vec<BrowseInfo>> {
        let input = serde_json::json!({
            "ids": ids.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
            "context": { "language": self.language, "country_code": self.country.to_uppercase() },
            "data_request": { "include_reviews": true, "include_tag_count": 12, "include_trailers": true }
        });
        let resp: Envelope<RawItems> = self
            .http
            .get(format!("{API}/IStoreBrowseService/GetItems/v1/"))
            .query(&[("input_json", input.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.response.store_items.into_iter().map(BrowseInfo::from).collect())
    }

    /// Localized names for all store tags.
    pub async fn tag_names(&self) -> Result<HashMap<u32, String>> {
        let resp: Envelope<RawTags> = self
            .http
            .get(format!("{API}/IStoreService/GetTagList/v1/"))
            .query(&[("language", self.language.as_str())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.response.tags.into_iter().map(|t| (t.tagid, t.name)).collect())
    }

    /// Latest news and patch notes for a game.
    pub async fn news(&self, id: AppId, count: u32) -> Result<Vec<NewsItem>> {
        let resp: RawNewsEnvelope = self
            .http
            .get(format!("{API}/ISteamNews/GetNewsForApp/v2/"))
            .query(&[("appid", id.to_string()), ("count", count.to_string()), ("maxlength", "400".into())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .appnews
            .newsitems
            .into_iter()
            .map(|n| NewsItem {
                title: n.title,
                url: n.url,
                author: n.author,
                excerpt: crate::html::text(&strip_bbcode(&n.contents)),
                date: n.date,
                feed: n.feedlabel,
            })
            .collect())
    }
}

/// Remove `[tags]` used in Steam announcements.
fn strip_bbcode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[derive(Deserialize)]
struct Envelope<T> {
    response: T,
}

#[derive(Deserialize)]
struct RawItems {
    #[serde(default)]
    store_items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    appid: AppId,
    reviews: Option<RawReviews>,
    #[serde(default)]
    tagids: Vec<u32>,
    trailers: Option<RawTrailers>,
    #[serde(default)]
    content_descriptorids: Vec<u32>,
}

#[derive(Deserialize)]
struct RawReviews {
    summary_filtered: Option<RawSummary>,
}

#[derive(Deserialize)]
struct RawSummary {
    #[serde(default)]
    review_count: u64,
    #[serde(default)]
    percent_positive: u32,
    #[serde(default)]
    review_score: u32,
    #[serde(default)]
    review_score_label: String,
}

#[derive(Deserialize)]
struct RawTrailers {
    #[serde(default)]
    highlights: Vec<RawTrailer>,
    #[serde(default)]
    other_trailers: Vec<RawTrailer>,
}

#[derive(Deserialize)]
struct RawTrailer {
    #[serde(default)]
    trailer_name: String,
    #[serde(default)]
    screenshot_full: String,
    #[serde(default)]
    microtrailer: Vec<RawFile>,
    #[serde(default)]
    adaptive_trailers: Vec<RawAdaptive>,
}

#[derive(Deserialize)]
struct RawFile {
    filename: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Deserialize)]
struct RawAdaptive {
    cdn_path: String,
    encoding: String,
}

impl From<RawItem> for BrowseInfo {
    fn from(r: RawItem) -> Self {
        let trailers = r
            .trailers
            .map(|t| t.highlights.into_iter().chain(t.other_trailers).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|t| Trailer {
                name: t.trailer_name,
                thumbnail: format!("{IMAGES}/{}", t.screenshot_full),
                micro_mp4: t.microtrailer.iter().find(|f| f.kind == "video/mp4").map(|f| format!("{VIDEO}/{}", f.filename)),
                hls: t.adaptive_trailers.iter().find(|a| a.encoding == "hls_h264").map(|a| format!("{VIDEO}/{}", a.cdn_path)),
            })
            .collect();
        BrowseInfo {
            app_id: r.appid,
            reviews: r.reviews.and_then(|r| r.summary_filtered).filter(|s| s.review_count > 0).map(|s| Reviews {
                label: s.review_score_label,
                percent_positive: s.percent_positive,
                count: s.review_count,
                score: s.review_score,
            }),
            tags: r.tagids,
            trailers,
            content_descriptors: r.content_descriptorids,
        }
    }
}

#[derive(Deserialize)]
struct RawTags {
    #[serde(default)]
    tags: Vec<RawTag>,
}

#[derive(Deserialize)]
struct RawTag {
    tagid: u32,
    name: String,
}

#[derive(Deserialize)]
struct RawNewsEnvelope {
    appnews: RawNews,
}

#[derive(Deserialize)]
struct RawNews {
    #[serde(default)]
    newsitems: Vec<RawNewsItem>,
}

#[derive(Deserialize)]
struct RawNewsItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    contents: String,
    #[serde(default)]
    date: u64,
    #[serde(default)]
    feedlabel: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_bbcode() {
        assert_eq!(super::strip_bbcode("[b]Patch[/b] notes [url=x]here[/url]"), "Patch notes here");
    }
}

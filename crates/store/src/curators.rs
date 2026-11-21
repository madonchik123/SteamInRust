//! Steam Curators: their pages and recommendations. Steam only serves these
//! as store pages and page fragments, so the few fields used are read from
//! its markup. Nothing here needs a sign-in.

use crate::{Error, Result, StoreClient, html};
use model::AppId;
use serde::Deserialize;

const STORE: &str = "https://store.steampowered.com";
const AVATARS: &str = "https://avatars.fastly.steamstatic.com";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curator {
    pub id: u32,
    pub name: String,
    pub avatar: String,
    pub description: String,
    pub followers: u64,
    pub reviews: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Recommended,
    NotRecommended,
    Informational,
}

/// One game a curator wrote about.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub app_id: AppId,
    pub name: String,
    pub art: String,
    pub verdict: Verdict,
    pub blurb: String,
    pub date: String,
    pub price: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picks {
    pub items: Vec<Pick>,
    pub total: u64,
}

impl StoreClient {
    fn locale_query(&self) -> [(&'static str, String); 2] {
        [("l", self.language.clone()), ("cc", self.country.clone())]
    }

    /// The most followed curators, `count` from `start`, and how many exist.
    pub async fn top_curators(&self, start: u32, count: u32) -> Result<(Vec<Curator>, u64)> {
        let raw: RawList = self
            .http
            .get(format!("{STORE}/curators/ajaxgetcurators/render/"))
            .query(&[("start", start.to_string()), ("count", count.to_string())])
            .query(&self.locale_query())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok((top_from(&raw.results_html), raw.total_count))
    }

    pub async fn curator(&self, id: u32) -> Result<Curator> {
        let page = self.http.get(format!("{STORE}/curator/{id}/")).query(&self.locale_query()).send().await?.error_for_status()?.text().await?;
        curator_from(&page, id).ok_or(Error::Unavailable)
    }

    /// A curator's reviews, newest first.
    pub async fn curator_picks(&self, id: u32, start: u32, count: u32) -> Result<Picks> {
        let q = [("query", String::new()), ("start", start.to_string()), ("count", count.to_string()), ("tagids", String::new()), ("sort", "recent".into()), ("types", "0".into())];
        let raw: RawList = self
            .http
            .get(format!("{STORE}/curator/{id}/ajaxgetfilteredrecommendations/render/"))
            .query(&q)
            .query(&self.locale_query())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Picks { items: picks_from(&raw.results_html), total: raw.total_count })
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawList {
    results_html: String,
    total_count: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTop {
    #[serde(rename = "clanID")]
    clan: serde_json::Value,
    name: String,
    #[serde(rename = "strAvatarHash")]
    avatar: String,
    total_followers: u64,
    total_reviews: u64,
    curator_description: String,
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &s[s.find(start)? + start.len()..];
    Some(&rest[..rest.find(end)?])
}

fn digits(s: &str) -> u64 {
    s.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)
}

/// The list page carries its curators as a script variable.
fn top_from(page: &str) -> Vec<Curator> {
    let Some(i) = page.find("g_rgTopCurators") else { return Vec::new() };
    let Some(eq) = page[i..].find('=') else { return Vec::new() };
    let json = page[i + eq + 1..].trim_start();
    let list = serde_json::Deserializer::from_str(json).into_iter::<Vec<RawTop>>().next();
    let Some(Ok(list)) = list else { return Vec::new() };
    list.into_iter()
        .filter_map(|c| {
            let id = match &c.clan {
                serde_json::Value::String(s) => s.parse().ok()?,
                v => u32::try_from(v.as_u64()?).ok()?,
            };
            Some(Curator {
                id,
                name: html::text(&c.name),
                avatar: if c.avatar.is_empty() { String::new() } else { format!("{AVATARS}/{}_full.jpg", c.avatar) },
                description: html::text(&c.curator_description),
                followers: c.total_followers,
                reviews: c.total_reviews,
            })
        })
        .collect()
}

fn curator_from(page: &str, id: u32) -> Option<Curator> {
    let head = &page[page.find("curator_name")?..];
    let name = html::text(between(&head[head.find("<a")?..], ">", "</a>")?);
    if name.is_empty() {
        return None;
    }
    Some(Curator {
        id,
        name,
        avatar: between(page, "class=\"curator_avatar\" src=\"", "\"").unwrap_or_default().to_string(),
        description: between(page, "og:description\" content=\"", "\"").map(html::text).unwrap_or_default(),
        followers: between(page, &format!("CuratorNumFollowers_{id}\">"), "<").map(digits).unwrap_or(0),
        reviews: between(page, "\"total_count\":", ",").map(digits).unwrap_or(0),
    })
}

fn picks_from(fragment: &str) -> Vec<Pick> {
    fragment
        .split("class=\"recommendation\"")
        .skip(1)
        .filter_map(|c| {
            let app_id = between(c, "data-ds-appid=\"", "\"")?.parse().ok()?;
            let verdict = if c.contains("ico_curator_up") || c.contains("color_recommended") {
                Verdict::Recommended
            } else if c.contains("ico_curator_down") || c.contains("color_not_recommended") {
                Verdict::NotRecommended
            } else {
                Verdict::Informational
            };
            Some(Pick {
                app_id,
                name: between(c, "alt=\"", "\"").map(html::text).unwrap_or_default(),
                art: between(c, "<img src=\"", "\"").unwrap_or_default().to_string(),
                verdict,
                blurb: between(c, "recommendation_desc\">", "</div>").map(html::text).unwrap_or_default(),
                date: between(c, "curator_review_date\">", "<").map(html::text).unwrap_or_default(),
                price: between(c, "discount_final_price\">", "<").map(html::text).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = r#"<div><div data-panel="{}" role="button" class="recommendation" ><div>
        <a data-ds-appid="264380" data-ds-itemkey="App_264380" class="store_capsule price_inline" href="x">
        <div class="capsule"><img src="https://img/264380/header.jpg" alt="Narcissu 1st &amp; 2nd"></div>
        <div class="discount_block"><div class="discount_prices"><div class="discount_final_price">Free</div></div></div></a></div>
        <a class="recommendation_link"><div class="recommendation_midcol"><div class="recommendation_stats"><div class="recommendation_type_ctn">
        <img src='https://x/ico_curator_up.png'> <span class='color_recommended'>Recommended</span>
        <span class="curator_review_date">22 September, 2014</span></div></div>
        <div class="recommendation_desc"> You read, you cry. </div></div></a></div>"#;

    #[test]
    fn reads_a_recommendation() {
        let p = picks_from(CARD);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].app_id, p[0].name.as_str(), p[0].verdict), (264380, "Narcissu 1st & 2nd", Verdict::Recommended));
        assert_eq!((p[0].art.as_str(), p[0].blurb.as_str(), p[0].date.as_str(), p[0].price.as_str()), ("https://img/264380/header.jpg", "You read, you cry.", "22 September, 2014", "Free"));
    }

    #[test]
    fn tells_the_verdicts_apart() {
        let info = CARD.replace("ico_curator_up", "ico_curator_info").replace("color_recommended", "color_informational");
        assert_eq!(picks_from(&info)[0].verdict, Verdict::Informational);
        let down = CARD.replace("ico_curator_up", "ico_curator_down").replace("color_recommended", "color_not_recommended");
        assert_eq!(picks_from(&down)[0].verdict, Verdict::NotRecommended);
    }

    #[test]
    fn skips_cards_that_arent_games() {
        assert!(picks_from(&CARD.replace("data-ds-appid", "data-ds-bundleid")).is_empty());
    }

    #[test]
    fn reads_a_curators_page() {
        let page = r#"<meta property="og:description" content="Story-driven &amp; retro games."><img class="curator_avatar" src="https://a/full.jpg"></a>
            <h2 class="pageheader curator_name"> <a data-panel="{}" href="https://x">Hanako&#39;s Shelves</a> </h2>
            <div class="num_followers" id="CuratorNumFollowers_6856153">1,212</div>
            g_pagingData = {"query":"","total_count":13,"pagesize":10}"#;
        let c = curator_from(page, 6856153).unwrap();
        assert_eq!((c.name.as_str(), c.avatar.as_str(), c.followers, c.reviews), ("Hanako's Shelves", "https://a/full.jpg", 1212, 13));
        assert_eq!(c.description, "Story-driven & retro games.");
        assert_eq!(curator_from("<html>no curator here</html>", 1), None);
    }

    #[test]
    fn reads_the_top_list() {
        let page = r#"<script> var g_nNumIgnoredCurators = 0; var g_rgTopCurators = [{"clanID":"1850","name":"PC Gamer","strAvatarHash":"34d4","total_reviews":921,"total_followers":731099,"curator_description":"We seek &amp; find.","m_rgAppRecommendations":[{"appid":1}]}]; var other = 1;</script>"#;
        let t = top_from(page);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].id, t[0].name.as_str(), t[0].followers, t[0].reviews), (1850, "PC Gamer", 731099, 921));
        assert_eq!(t[0].avatar, "https://avatars.fastly.steamstatic.com/34d4_full.jpg");
        assert_eq!(t[0].description, "We seek & find.");
        assert!(top_from("<div>nothing</div>").is_empty());
    }
}

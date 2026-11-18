//! What players wrote about a game: the reviews on its community hub
//! (Steam's store links there too), most helpful or newest, ten a page.
//! Steam's old `appreviews` JSON now redirects to the store's front page.

use crate::{AppId, Result, StoreClient};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Review {
    /// Link to the review (also identifies it).
    pub id: String,
    pub author: String,
    /// The author's SteamID (0 if the page didn't say).
    pub author_id: u64,
    /// e.g. "431 products in account".
    pub products: String,
    pub avatar: String,
    /// e.g. "28.5 hrs on record".
    pub hours: String,
    /// e.g. "Posted: 18 August".
    pub posted: String,
    pub text: String,
    pub voted_up: bool,
    /// e.g. "451 people found this review helpful".
    pub helpful: String,
    /// e.g. "1 person found this review funny".
    pub funny: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReviewPage {
    pub reviews: Vec<Review>,
    /// There may be more on the next page.
    pub more: bool,
    /// Where the next page is (Steam's "more content" form, with its
    /// cursor): asking for `p=2` just repeats the first page.
    pub next: Option<String>,
}

/// Which reviews: Steam's "Most helpful", the newest first, or the funniest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Order {
    #[default]
    Helpful,
    Recent,
    Funny,
}

/// Recommended, not recommended, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    #[default]
    All,
    Positive,
    Negative,
}

/// How far back "Most helpful" looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Window {
    #[default]
    AllTime,
    Day,
    Week,
    Month,
    ThreeMonths,
    SixMonths,
    Year,
}

/// The filters on Steam's reviews (the Community hub's own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Filter {
    pub kind: Kind,
    /// Every language, not just yours.
    pub all_languages: bool,
    pub window: Window,
}

impl Filter {
    /// The hub page and its `browsefilter` for an order.
    fn path_and_sort(&self, order: Order) -> (&'static str, &'static str) {
        let path = match self.kind {
            Kind::All => "reviews",
            Kind::Positive => "positivereviews",
            Kind::Negative => "negativereviews",
        };
        let sort = match order {
            Order::Recent => "mostrecent",
            Order::Funny => "funny",
            Order::Helpful => match self.window {
                Window::AllTime => "toprated",
                Window::Day => "trendday",
                Window::Week => "trendweek",
                Window::Month => "trendmonth",
                Window::ThreeMonths => "trendthreemonths",
                Window::SixMonths => "trendsixmonths",
                Window::Year => "trendyear",
            },
        };
        (path, sort)
    }
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)?;
    Some(&s[i..i + j])
}

/// HTML to plain text: tags dropped, `<br>` as line breaks, entities decoded.
fn clean(html: &str) -> String {
    let html = html.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&amp;", "&");
    out.lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub(crate) fn parse(body: &str) -> ReviewPage {
    let reviews: Vec<Review> = body
        .split("class=\"apphub_Card modalContentLink")
        .skip(1)
        .filter_map(|card| {
            let id = between(card, "data-modal-content-url=\"", "\"")?.to_string();
            let helpful_block = between(card, "class=\"found_helpful\">", "<div class=\"review_award").unwrap_or_default();
            let mut lines = helpful_block.split("<br>").map(clean).filter(|l| !l.is_empty());
            let (helpful, funny) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
            let content = between(card, "class=\"apphub_CardTextContent\">", "<div class=\"UserReviewCardContent_Footer").unwrap_or_default();
            let posted = between(content, "class=\"date_posted\">", "</div>").map(clean).unwrap_or_default();
            // The text follows the date (an Early Access note may sit between).
            let after_date = content.split_once("class=\"date_posted\">").and_then(|(_, rest)| rest.split_once("</div>")).map(|(_, rest)| rest).unwrap_or(content);
            let text = match after_date.split_once("<div class=\"early_access_review\">") {
                Some((_, rest)) => rest.split_once("</div>").map(|(_, t)| t).unwrap_or(rest),
                None => after_date,
            };
            let author_block = card.split("class=\"apphub_CardContentAuthorName").nth(1).unwrap_or_default();
            Some(Review {
                id,
                author: between(author_block, "\">", "</a>").map(clean).unwrap_or_default(),
                author_id: between(card, "data-miniprofile=\"", "\"").and_then(|a| a.parse::<u64>().ok()).map(|a| 76_561_197_960_265_728 + a).unwrap_or(0),
                products: between(card, "class=\"apphub_CardContentMoreLink ellipsis\">", "</div>").map(clean).unwrap_or_default(),
                avatar: between(card, "class=\"appHubIconHolder", "</div>").and_then(|b| between(b, "<img src=\"", "\"")).unwrap_or_default().to_string(),
                hours: between(card, "class=\"hours\">", "<").map(clean).unwrap_or_default(),
                posted,
                text: clean(text),
                voted_up: !card.contains("icon_thumbsDown"),
                helpful,
                funny,
            })
        })
        .collect();
    let next = next_page(body);
    let more = !reviews.is_empty() && next.is_some();
    ReviewPage { reviews, more, next }
}

/// The address of the next page: the hub's hidden "MoreContentForm" (its
/// action and every field, the cursor among them).
fn next_page(body: &str) -> Option<String> {
    let start = body.find("id=\"MoreContentForm")?;
    let form = &body[start..];
    let form = &form[..form.find("</form>")?];
    let action = between(form, "action=\"", "\"")?;
    let mut url = reqwest::Url::parse(&action.replace("&amp;", "&")).ok()?;
    {
        let mut q = url.query_pairs_mut();
        for input in form.split("<input").skip(1) {
            let (Some(name), Some(value)) = (between(input, "name=\"", "\""), between(input, "value=\"", "\"")) else { continue };
            q.append_pair(name, &value.replace("&amp;", "&"));
        }
    }
    url.query_pairs().any(|(k, _)| k == "userreviewscursor").then(|| url.to_string())
}

impl StoreClient {
    /// Page `page` (from 1) of a game's reviews, filtered like the hub.
    pub async fn reviews(&self, app: AppId, order: Order, filter: Filter, page: u32) -> Result<ReviewPage> {
        let (path, sort) = filter.path_and_sort(order);
        let mut url = reqwest::Url::parse(&format!("https://steamcommunity.com/app/{app}/{path}/")).expect("valid url");
        url.query_pairs_mut()
            .append_pair("browsefilter", sort)
            .append_pair("filterLanguage", if filter.all_languages { "all" } else { &self.language })
            .append_pair("l", &self.language)
            .append_pair("p", &page.max(1).to_string())
            .append_pair("appHubSubSection", "10");
        let body = self.http.get(url).send().await?.error_for_status()?.text().await?;
        Ok(parse(&body))
    }

    /// The page after one already loaded (its [`ReviewPage::next`]).
    pub async fn reviews_next(&self, next: &str) -> Result<ReviewPage> {
        let body = self.http.get(next).send().await?.error_for_status()?.text().await?;
        Ok(parse(&body))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn filters_pick_the_hub_page() {
        use super::{Filter, Kind, Order, Window};
        let f = Filter { kind: Kind::Negative, window: Window::Week, ..Default::default() };
        assert_eq!(f.path_and_sort(Order::Helpful), ("negativereviews", "trendweek"));
        assert_eq!(f.path_and_sort(Order::Recent), ("negativereviews", "mostrecent"));
        assert_eq!(Filter::default().path_and_sort(Order::Funny), ("reviews", "funny"));
    }

    #[test]
    fn reads_hub_review_cards() {
        let body = r#"<div class="apphub_Card modalContentLink interactable" style="display: none" data-modal-content-url="https://steamcommunity.com/profiles/76561199579272739/recommended/570/" data-modal-content-sizetofit="false">
            <div class="apphub_CardContentMain"><div class="apphub_UserReviewCardContent">
            <div class="found_helpful"> 451 people found this review helpful<br>1 person found this review funny <div class="review_award_aggregated tooltip">0</div></div>
            <div class="vote_header"><div class="reviewInfo"><div class="thumb"><img src="https://x/icon_thumbsUp.png?v=1"></div><div class="title">Recommended</div><div class="hours">28.5 hrs on record</div></div></div>
            <div class="apphub_CardTextContent"><div class="date_posted">Posted: 18 August</div>
            Good game but very harsh &amp; <b>strict</b><br>second line </div></div>
            <div class="UserReviewCardContent_Footer"><div class="gradient">&nbsp;</div></div></div>
            <div class="apphub_CardContentAuthorBlock tall"><div class="apphub_friend_block_container"><a href="x"><div class="apphub_friend_block" data-miniprofile="1">
            <div class="appHubIconHolder offline"><img src="https://avatars.fastly.steamstatic.com/a.jpg"></div>
            <div class="apphub_CardContentAuthorName offline ellipsis"><a href="https://steamcommunity.com/profiles/1/">Mr. Moe</a></div></div></a></div></div></div>
            <div class="apphub_Card modalContentLink interactable" data-modal-content-url="https://steamcommunity.com/id/b/recommended/570/">
            <div class="found_helpful">No one has rated this review as helpful yet<div class="review_award_aggregated"></div></div><img src="icon_thumbsDown.png"><div class="hours">1.0 hrs on record</div>
            <div class="apphub_CardTextContent"><div class="date_posted">Posted: 1 May</div><div class="early_access_review">Early Access Review</div>Too early</div><div class="UserReviewCardContent_Footer"></div></div>"#;
        let p = super::parse(body);
        assert_eq!(p.reviews.len(), 2);
        let r = &p.reviews[0];
        assert_eq!((r.author.as_str(), r.hours.as_str(), r.posted.as_str(), r.voted_up), ("Mr. Moe", "28.5 hrs on record", "Posted: 18 August", true));
        assert_eq!(r.text, "Good game but very harsh & strict\nsecond line");
        assert_eq!((r.helpful.as_str(), r.funny.as_str()), ("451 people found this review helpful", "1 person found this review funny"));
        assert_eq!(r.avatar, "https://avatars.fastly.steamstatic.com/a.jpg");
        assert_eq!(r.author_id, 76_561_197_960_265_729);
        let r = &p.reviews[1];
        assert_eq!((r.voted_up, r.text.as_str(), r.helpful.as_str()), (false, "Too early", "No one has rated this review as helpful yet"));
        assert!(!p.more);
    }
}

impl StoreClient {
    /// Up and down votes from the last 30 days (Steam's review histogram,
    /// which feeds "Recent reviews" on its store page).
    pub async fn recent_votes(&self, app: AppId) -> Result<(u64, u64)> {
        let url = format!("https://store.steampowered.com/appreviewhistogram/{app}?l={}&review_score_preference=0", self.language);
        let v: serde_json::Value = self.http.get(url).send().await?.error_for_status()?.json().await?;
        Ok(sum_recent(&v))
    }
}

fn sum_recent(v: &serde_json::Value) -> (u64, u64) {
    let days = v.pointer("/results/recent").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    days.iter().fold((0, 0), |(u, d), day| {
        (u + day.get("recommendations_up").and_then(|x| x.as_u64()).unwrap_or(0), d + day.get("recommendations_down").and_then(|x| x.as_u64()).unwrap_or(0))
    })
}

#[cfg(test)]
mod histogram_tests {
    #[test]
    fn sums_the_last_30_days() {
        let v: serde_json::Value = serde_json::from_str(r#"{"success":1,"results":{"recent":[{"date":1,"recommendations_up":427,"recommendations_down":219},{"date":2,"recommendations_up":3,"recommendations_down":1}]}}"#).unwrap();
        assert_eq!(super::sum_recent(&v), (430, 220));
        assert_eq!(super::sum_recent(&serde_json::Value::Null), (0, 0));
    }

    #[test]
    fn finds_the_next_page() {
        let body = r#"<form method="GET" id="MoreContentForm1" name="MoreContentForm1" action="https://steamcommunity.com/app/570/homecontent/">
            <input type="hidden" name="userreviewscursor" value="AoIIP3XT">
            <input type="hidden" name="p" value="2"><input type="hidden" name="browsefilter" value="toprated"></form>"#;
        let next = super::next_page(body).expect("a next page");
        assert!(next.starts_with("https://steamcommunity.com/app/570/homecontent/?"));
        assert!(next.contains("userreviewscursor=AoIIP3XT") && next.contains("p=2") && next.contains("browsefilter=toprated"));
        assert_eq!(super::next_page("<div>no form</div>"), None);
    }
}

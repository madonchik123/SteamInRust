//! Social feeds for the Community home: your friends' activity (Steam's
//! activity feed, which the official client no longer links to but still
//! serves at `/my/ajaxgetusernews/`) and Steam's community-wide feed of
//! popular and recent posts (`/apps/allcontenthome`). Read-only.

use crate::market::Web;
pub use parse::{parse_activity, parse_explore};

mod parse;
use model::SteamId;

const COMMUNITY: &str = "https://steamcommunity.com";

/// A page of posts.
#[derive(Debug, Clone, Default)]
pub struct Feed {
    pub posts: Vec<Post>,
    /// Start time of the next (older) page; 0 = no more.
    pub next: u64,
}

/// What a post is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PostKind {
    #[default]
    Other,
    Screenshot,
    Artwork,
    Video,
    Broadcast,
    Workshop,
    Guide,
    Review,
    Achievement,
    /// Bought or got a game.
    Purchase,
    /// Became friends, joined a group, a status post…
    Status,
    /// A game's announcement (from games you follow, or on its hub).
    Announcement,
    /// A topic in a game's discussions.
    Discussion,
}

#[derive(Debug, Clone, Default)]
pub struct Post {
    pub kind: PostKind,
    pub author: SteamId,
    pub author_name: String,
    pub avatar: Option<String>,
    pub app_id: model::AppId,
    pub game: String,
    /// What they did, for your friends' activity ("posted a screenshot of
    /// …", or a whole line like "X and Y are now friends.").
    pub action: String,
    /// The post's own title or caption (a screenshot's caption, a guide's
    /// or an announcement's title).
    pub title: String,
    /// Caption, review text or description.
    pub text: String,
    /// Pictures (a screenshot, artwork, achievement icons…).
    pub images: Vec<String>,
    /// Steam link to the post.
    pub url: String,
    /// When, as Steam wrote it ("12 Sep", "Posted: November 12").
    pub when: String,
    /// When, as a Unix time, if known (shared files).
    pub time: u64,
    /// Recommended (true) or not (false), for reviews.
    pub recommended: Option<bool>,
    /// Hours on record, for reviews.
    pub hours: String,
    pub votes_up: u32,
    pub funny: u32,
    pub comments: u32,
    pub views: u32,
    pub live: bool,
    /// The Workshop / screenshot id, when it's a shared file.
    pub file_id: u64,
}

/// Sections of the community-wide feed, as on Steam's community home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Section {
    #[default]
    All,
    Screenshots,
    Artwork,
    Broadcasts,
    Videos,
    Workshop,
    Guides,
    Reviews,
}

impl Section {
    pub const ALL: [Section; 8] = [Section::All, Section::Screenshots, Section::Artwork, Section::Broadcasts, Section::Videos, Section::Workshop, Section::Guides, Section::Reviews];

    /// Steam's `appHubSubSection` number (a game's hub calls "all" 1).
    fn number(self, app: bool) -> u32 {
        match self {
            Section::All if app => 1,
            Section::All => 11,
            Section::Screenshots => 2,
            Section::Artwork => 4,
            Section::Broadcasts => 13,
            Section::Videos => 3,
            Section::Workshop => 6,
            Section::Guides => 9,
            Section::Reviews => 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ExploreQuery {
    pub section: Section,
    /// Most recent instead of most popular.
    pub recent: bool,
    pub page: u32,
    pub language: String,
    /// One game's hub instead of the whole community (0 = everyone's).
    pub app: model::AppId,
    /// Search words (a game's hub only).
    pub search: String,
}

impl ExploreQuery {
    pub fn key(&self) -> String {
        format!("{self:?}")
    }
}


/// A page of your friends' activity.
pub async fn activity(web: &Web, me: SteamId, start: u64, language: &str) -> Result<Feed, String> {
    let mut url = reqwest::Url::parse(&format!("{COMMUNITY}/profiles/{me}/ajaxgetusernews/")).map_err(|e| e.to_string())?;
    if start > 0 {
        url.query_pairs_mut().append_pair("start", &start.to_string());
    }
    url.query_pairs_mut().append_pair("l", language);
    let body = crate::market::page(web, url.as_str()).await?;
    parse_activity(&body)
}

/// A page of Steam's community-wide feed, or of one game's hub.
pub async fn explore(web: &Web, q: &ExploreQuery) -> Result<Feed, String> {
    let page = q.page.max(1).to_string();
    let mut url = if q.app != 0 {
        reqwest::Url::parse(&format!("{COMMUNITY}/app/{}/homecontent/", q.app)).map_err(|e| e.to_string())?
    } else {
        reqwest::Url::parse(&format!("{COMMUNITY}/apps/allcontenthome")).map_err(|e| e.to_string())?
    };
    url.query_pairs_mut()
        .append_pair("l", &q.language)
        .append_pair("browsefilter", if q.recent { "mostrecent" } else { "trend" })
        .append_pair("appHubSubSection", &q.section.number(q.app != 0).to_string())
        .append_pair("forceanon", "1")
        .append_pair("p", &page);
    if q.app != 0 {
        // A game's hub pages each kind of content on its own counter.
        url.query_pairs_mut().append_pair("numperpage", "10").append_pair("filterLanguage", "default").append_pair("searchText", q.search.trim());
        for counter in ["screenshotspage", "videospage", "artpage", "allguidepage", "webguidepage", "integratedguidepage", "discussionspage", "workshopitemspage", "readytouseitemspage", "mtxitemspage", "itemspage"] {
            url.query_pairs_mut().append_pair(counter, &page);
        }
        url.query_pairs_mut().append_pair("userreviewsoffset", &((q.page.max(1) - 1) * 3).to_string());
    }
    let body = crate::market::page(web, url.as_str()).await?;
    Ok(Feed { posts: parse_explore(&body), next: 0 })
}

/// Fill in a shared file's post from Steam's details of it: the author
/// and title when the page left them out, when it was made, and what it
/// is (Steam's `EWorkshopFileType`; the page only says it in words, in the
/// reader's language).
pub fn fill_from_details(p: &mut Post, creator: SteamId, file_type: u32, title: &str, created: u64) {
    if p.author == 0 {
        p.author = creator;
    }
    if p.title.is_empty() {
        p.title = title.to_string();
    }
    if p.time == 0 {
        p.time = created;
    }
    let kind = match file_type {
        3 => Some(PostKind::Artwork),
        4 => Some(PostKind::Video),
        5 => Some(PostKind::Screenshot),
        9 | 10 => Some(PostKind::Guide),
        0..=2 => Some(PostKind::Workshop),
        _ => None,
    };
    // A guide card keeps its own layout; pictures get their true kind.
    if let Some(k) = kind.filter(|_| matches!(p.kind, PostKind::Screenshot | PostKind::Artwork | PostKind::Workshop | PostKind::Video | PostKind::Other)) {
        p.kind = k;
    }
}

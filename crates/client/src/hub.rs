//! Community content for the native hub pages: Workshop items, guides,
//! discussions, a game's community hub and achievement comparisons.
//!
//! Data comes from Steam's unified services where they exist
//! (`PublishedFile.*`, `Player.*`) and from steamcommunity.com pages where
//! they don't (discussion forums, guide sections, comments). Everything here
//! only reads, except [`Request::Vote`] and [`Request::Favorite`], which the
//! user starts with a button.

use crate::market::Web;
use crate::{Event, Sink};
use model::{AppId, SteamId};
use net::Connection;
use proto::*;
use std::collections::HashMap;
use std::sync::Arc;

const COMMUNITY: &str = "https://steamcommunity.com";
const BASE_ID: u64 = 76561197960265728;

/// Content descriptors left out of lists unless mature content is turned on
/// (sexual content; violence alone is shown, like Steam's default).
pub const MATURE: [i32; 3] = [1, 3, 4];

/// What to ask Steam for. Send it as [`crate::Command::Hub`]; the answer
/// comes back as [`crate::Event::Hub`] and lands in [`Cache`].
#[derive(Debug, Clone)]
pub enum Request {
    /// One Workshop item or guide: previews, required items, change notes,
    /// your vote.
    Item { file_id: u64, language: String },
    /// A page of a game's Workshop items, guides, screenshots, artwork or videos.
    Files(FileQuery),
    /// A page of what someone shared (their screenshots, Workshop items…),
    /// newest first. Answered as [`Response::Files`] with [`user_files_key`].
    UserFiles { steam_id: SteamId, kind: FileKind, page: u32, language: String },
    /// A guide's sections, read from its page.
    Guide { file_id: u64, language: String },
    /// Comments under a Workshop item or guide.
    FileComments { file_id: u64, creator: SteamId, start: u32 },
    /// A page of a discussion forum. `forum` is the forum's path after
    /// `/app/<id>/` (`discussions/0/`, `tradingforum/`); empty = the game's
    /// main forum. Also lists the game's other forums.
    Topics { app_id: AppId, forum: String, page: u32, language: String },
    /// A page of a discussion thread (`url` is the thread's Steam link).
    Thread { url: String, page: u32, language: String },
    /// A game's news posts.
    News { app_id: AppId, count: u32 },
    /// Someone's achievements in a game (to compare with yours).
    Achievements { steam_id: SteamId, app_id: AppId, language: String },
    /// Rate an item or guide up or down.
    Vote { file_id: u64, up: bool },
    /// Add an item or guide to your favorites, or take it out.
    Favorite { app_id: AppId, file_id: u64, add: bool },
}

#[derive(Debug, Clone)]
pub enum Response {
    Item { file_id: u64, result: Result<Item, String> },
    Files { key: String, result: Result<FilePage, String> },
    Guide { file_id: u64, result: Result<Guide, String> },
    FileComments { file_id: u64, result: Result<model::Comments, String> },
    Topics { key: String, result: Result<TopicPage, String> },
    Thread { key: String, result: Result<Thread, String> },
    News { app_id: AppId, result: Result<Vec<News>, String> },
    Achievements { steam_id: SteamId, app_id: AppId, result: Result<Vec<model::Achievement>, String> },
    Voted { file_id: u64, up: bool, result: Result<(), String> },
    Favorited { file_id: u64, add: bool, result: Result<(), String> },
}

/// Kinds of shared files a game can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileKind {
    #[default]
    Items,
    Collections,
    Art,
    Videos,
    Screenshots,
    Guides,
}

impl FileKind {
    /// Steam's EPublishedFileInfoMatchingFileType.
    fn matching(self) -> u32 {
        match self {
            FileKind::Items => 0,
            FileKind::Collections => 1,
            FileKind::Art => 2,
            FileKind::Videos => 3,
            FileKind::Screenshots => 4,
            FileKind::Guides => 11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FileQuery {
    pub app_id: AppId,
    pub kind: FileKind,
    /// Steam's EPublishedFileQueryType (0 top rated, 1 newest, 3 trending,
    /// 9 most subscribed, 12 text search, 21 recently updated).
    pub sort: u32,
    /// For trending: how many days back (0 = Steam's default).
    pub days: u32,
    pub page: u32,
    pub per_page: u32,
    pub search: String,
    pub tags: Vec<String>,
    pub language: String,
    /// Include sexual content.
    pub mature: bool,
}

impl FileQuery {
    /// Cache key for this query.
    pub fn key(&self) -> String {
        format!("{self:?}")
    }
}

/// One entry in a list of shared files.
#[derive(Debug, Clone, Default)]
pub struct FileEntry {
    pub id: u64,
    pub app_id: AppId,
    pub title: String,
    pub preview: Option<String>,
    /// Full-size image (screenshots, artwork).
    pub image: Option<String>,
    pub youtube: Option<String>,
    pub description: String,
    pub creator: SteamId,
    pub size: u64,
    pub subscriptions: u64,
    pub favorited: u64,
    pub views: u64,
    pub created: u64,
    pub updated: u64,
    pub votes_up: u32,
    pub votes_down: u32,
    pub score: f32,
    pub comments: u32,
    pub tags: Vec<String>,
    /// Flagged by Steam as sexual content.
    pub mature: bool,
    pub spoiler: bool,
}

impl FileEntry {
    /// Steam's star rating, 0–5 (`None` = not enough ratings yet).
    pub fn stars(&self) -> Option<u8> {
        stars(self.votes_up, self.votes_down)
    }
}

/// Steam shows stars once an item has a few ratings.
pub fn stars(up: u32, down: u32) -> Option<u8> {
    let total = up + down;
    if total < 3 {
        return None;
    }
    Some((up as f32 / total as f32 * 5.0).round().clamp(1.0, 5.0) as u8)
}

#[derive(Debug, Clone, Default)]
pub struct FilePage {
    pub total: u64,
    pub items: Vec<FileEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct Preview {
    pub url: Option<String>,
    pub youtube: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ChangeNote {
    pub time: u64,
    pub text: String,
}

/// A Workshop item or guide with everything its page shows.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub entry: FileEntry,
    pub app_name: String,
    /// Description in Steam's BBCode.
    pub description: String,
    pub previews: Vec<Preview>,
    /// Items this one needs (title, preview), in Steam's order.
    pub required: Vec<FileEntry>,
    pub change_notes: Vec<ChangeNote>,
    pub change_total: u32,
    pub lifetime_subscriptions: u64,
    pub file_type: u32,
    /// Your vote: `Some(true)` up, `Some(false)` down.
    pub my_vote: Option<bool>,
    pub subscribed: bool,
    pub banned: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    pub title: String,
    /// Steam's HTML.
    pub html: String,
}

#[derive(Debug, Clone, Default)]
pub struct Guide {
    pub title: String,
    pub authors: String,
    pub overview: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forum {
    pub name: String,
    /// Path after `/app/<id>/`, e.g. `discussions/0/`.
    pub path: String,
    pub threads: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Topic {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub replies: u32,
    pub last_post: u64,
    pub pinned: bool,
    pub locked: bool,
    /// The first lines of the opening post.
    pub preview: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct TopicPage {
    pub forum: String,
    pub total: u64,
    pub page_size: u32,
    pub topics: Vec<Topic>,
    pub forums: Vec<Forum>,
}

#[derive(Debug, Clone, Default)]
pub struct Post {
    pub id: u64,
    pub author: SteamId,
    pub name: String,
    pub avatar: Option<String>,
    pub time: u64,
    /// Steam's HTML.
    pub html: String,
}

#[derive(Debug, Clone, Default)]
pub struct Thread {
    pub title: String,
    pub forum: String,
    pub forum_url: String,
    pub op: Post,
    pub posts: Vec<Post>,
    pub total: u32,
    pub page_size: u32,
    pub locked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct News {
    pub id: String,
    pub title: String,
    pub url: String,
    pub author: String,
    /// BBCode or HTML.
    pub contents: String,
    pub feed: String,
    pub time: u64,
    pub external: bool,
}

pub fn user_files_key(steam_id: SteamId, kind: FileKind, page: u32) -> String {
    format!("user{steam_id}/{kind:?}#{page}")
}

pub fn topics_key(app_id: AppId, forum: &str, page: u32) -> String {
    format!("{app_id}/{forum}#{page}")
}

pub fn thread_key(url: &str, page: u32) -> String {
    format!("{url}#{page}")
}

/// Everything loaded so far, kept in [`crate::State::hub`].
#[derive(Debug, Clone, Default)]
pub struct Cache {
    pub items: HashMap<u64, Result<Item, String>>,
    pub files: HashMap<String, Result<FilePage, String>>,
    pub guides: HashMap<u64, Result<Guide, String>>,
    pub comments: HashMap<u64, Result<model::Comments, String>>,
    pub topics: HashMap<String, Result<TopicPage, String>>,
    pub threads: HashMap<String, Result<Thread, String>>,
    pub news: HashMap<AppId, Result<Vec<News>, String>>,
    pub achievements: HashMap<(SteamId, AppId), Result<Vec<model::Achievement>, String>>,
    /// Favorites you changed this session.
    pub favorites: HashMap<u64, bool>,
}

impl Cache {
    pub fn apply(&mut self, r: &Response) {
        match r {
            Response::Item { file_id, result } => {
                self.items.insert(*file_id, result.clone());
            }
            Response::Files { key, result } => {
                self.files.insert(key.clone(), result.clone());
            }
            Response::Guide { file_id, result } => {
                self.guides.insert(*file_id, result.clone());
            }
            Response::FileComments { file_id, result } => {
                self.comments.insert(*file_id, result.clone());
            }
            Response::Topics { key, result } => {
                self.topics.insert(key.clone(), result.clone());
            }
            Response::Thread { key, result } => {
                self.threads.insert(key.clone(), result.clone());
            }
            Response::News { app_id, result } => {
                self.news.insert(*app_id, result.clone());
            }
            Response::Achievements { steam_id, app_id, result } => {
                self.achievements.insert((*steam_id, *app_id), result.clone());
            }
            Response::Voted { file_id, up, result: Ok(()) } => {
                if let Some(Ok(item)) = self.items.get_mut(file_id) {
                    // Move our vote over in the counts.
                    match item.my_vote {
                        Some(true) => item.entry.votes_up = item.entry.votes_up.saturating_sub(1),
                        Some(false) => item.entry.votes_down = item.entry.votes_down.saturating_sub(1),
                        None => {}
                    }
                    if *up {
                        item.entry.votes_up += 1;
                    } else {
                        item.entry.votes_down += 1;
                    }
                    item.my_vote = Some(*up);
                }
            }
            Response::Favorited { file_id, add, result: Ok(()) } => {
                self.favorites.insert(*file_id, *add);
            }
            Response::Voted { .. } | Response::Favorited { .. } => {}
        }
    }
}

/// Steam's ELanguage number for a language name (`english`, `russian`).
pub fn language_id(name: &str) -> i32 {
    match name {
        "german" => 1,
        "french" => 2,
        "italian" => 3,
        "koreana" => 4,
        "spanish" => 5,
        "schinese" => 6,
        "tchinese" => 7,
        "russian" => 8,
        "japanese" => 10,
        "portuguese" => 11,
        "polish" => 14,
        "ukrainian" => 22,
        _ => 0,
    }
}

/// Items per page of someone's files.
pub const USER_PAGE: u32 = 30;

const PERSONA_FLAGS: u32 = 1 | 2 | 16 | 64 | 256 | 4096;

/// Ask Steam for names and avatars (they arrive as persona updates).
fn want_personas(conn: &Connection, ids: Vec<SteamId>) {
    let mut ids: Vec<SteamId> = ids.into_iter().filter(|i| *i > BASE_ID).collect();
    ids.sort_unstable();
    ids.dedup();
    if !ids.is_empty() {
        let _ = conn.send(net::emsg::CLIENT_REQUEST_FRIEND_DATA, &CMsgClientRequestFriendData { persona_state_requested: Some(PERSONA_FLAGS), friends: ids });
    }
}

/// Run one request; called by the session with the signed-in web session.
pub(crate) async fn handle(conn: Connection, me: SteamId, web: Result<Arc<Web>, String>, req: Request, sink: Sink) {
    let anon = Arc::new(Web::anonymous());
    let web_or_anon = web.clone().unwrap_or(anon);
    let out = |r: Response| sink(Event::Hub(r));
    match req {
        Request::Item { file_id, language } => {
            let result = item(&conn, me, file_id, &language).await;
            if let Ok(i) = &result {
                let mut ids = vec![i.entry.creator];
                ids.extend(i.required.iter().map(|r| r.creator));
                want_personas(&conn, ids);
            }
            out(Response::Item { file_id, result });
        }
        Request::Files(q) => {
            let key = q.key();
            let result = files(&conn, &q).await;
            if let Ok(p) = &result {
                want_personas(&conn, p.items.iter().map(|i| i.creator).collect());
            }
            out(Response::Files { key, result });
        }
        Request::UserFiles { steam_id, kind, page, language } => {
            let key = user_files_key(steam_id, kind, page);
            let req = CPublishedFileGetUserFilesRequest {
                steamid: Some(steam_id),
                appid: Some(0),
                page: Some(page.max(1)),
                numperpage: Some(USER_PAGE),
                r#type: Some("myfiles".into()),
                sortmethod: Some("created".into()),
                filetype: Some(kind.matching()),
                language: Some(language_id(&language)),
                excluded_content_descriptors: MATURE.to_vec(),
                return_vote_data: Some(true),
                return_tags: Some(true),
                return_previews: Some(true),
                return_short_description: Some(true),
                ..Default::default()
            };
            let result = match conn.call::<_, CPublishedFileGetUserFilesResponse>("PublishedFile.GetUserFiles#1", &req).await {
                Ok(r) => Ok(FilePage { total: r.total() as u64, items: r.publishedfiledetails.iter().filter(|d| d.result() == 1).map(entry).filter(|e| !e.mature).collect() }),
                Err(net::Error::EResult(code, _)) if code == net::eresult::ACCESS_DENIED => Err("private".to_string()),
                Err(e) => Err(e.to_string()),
            };
            out(Response::Files { key, result });
        }
        Request::Guide { file_id, language } => {
            let url = format!("{COMMUNITY}/sharedfiles/filedetails/?id={file_id}&l={language}");
            let result = crate::market::page(&web_or_anon, &url).await.and_then(|b| parse_guide(&b));
            out(Response::Guide { file_id, result });
        }
        Request::FileComments { file_id, creator, start } => {
            let result = file_comments(&web_or_anon, file_id, creator, start).await;
            out(Response::FileComments { file_id, result });
        }
        Request::Topics { app_id, forum, page, language } => {
            let key = topics_key(app_id, &forum, page);
            let path = if forum.is_empty() { "discussions/".to_string() } else { forum };
            let result = if path.contains("..") || path.contains('?') || path.contains("://") {
                Err("not a forum".to_string())
            } else {
                let url = format!("{COMMUNITY}/app/{app_id}/{path}?fp={}&l={language}", page.max(1));
                crate::market::page(&web_or_anon, &url).await.and_then(|b| parse_topics(&b, app_id))
            };
            out(Response::Topics { key, result });
        }
        Request::Thread { url, page, language } => {
            let key = thread_key(&url, page);
            let result = if !url.starts_with(COMMUNITY) || url.contains('?') {
                Err("not a Steam discussion".to_string())
            } else {
                let full = format!("{url}?ctp={}&l={language}", page.max(1));
                crate::market::page(&web_or_anon, &full).await.and_then(|b| parse_thread(&b))
            };
            out(Response::Thread { key, result });
        }
        Request::News { app_id, count } => {
            out(Response::News { app_id, result: news(app_id, count).await });
        }
        Request::Achievements { steam_id, app_id, language } => {
            let result = achievements(&conn, steam_id, app_id, &language).await;
            out(Response::Achievements { steam_id, app_id, result });
        }
        Request::Vote { file_id, up } => {
            let r: Result<CPublishedFileVoteResponse, _> = conn.call("PublishedFile.Vote#1", &CPublishedFileVoteRequest { publishedfileid: Some(file_id), vote_up: Some(up) }).await;
            out(Response::Voted { file_id, up, result: r.map(|_| ()).map_err(|e| e.to_string()) });
        }
        Request::Favorite { app_id, file_id, add } => {
            let result = match web {
                Ok(w) => favorite(&w, app_id, file_id, add).await,
                Err(e) => Err(e),
            };
            out(Response::Favorited { file_id, add, result });
        }
    }
}

fn entry(d: &PublishedFileDetails) -> FileEntry {
    let votes = d.vote_data.as_ref();
    let youtube = Some(d.youtubevideoid().to_string()).filter(|s| !s.is_empty());
    FileEntry {
        id: d.publishedfileid(),
        app_id: d.consumer_appid(),
        title: d.title().to_string(),
        preview: Some(d.preview_url().to_string()).filter(|p| !p.is_empty()),
        image: Some(d.image_url().to_string()).filter(|p| !p.is_empty()).or_else(|| Some(d.file_url().to_string()).filter(|p| p.starts_with("http") && d.file_type() == 5)),
        youtube,
        description: d.short_description().to_string(),
        creator: d.creator(),
        size: d.file_size(),
        subscriptions: d.subscriptions() as u64,
        favorited: d.favorited() as u64,
        views: d.views() as u64,
        created: d.time_created() as u64,
        updated: d.time_updated() as u64,
        votes_up: votes.map(|v| v.votes_up()).unwrap_or(0),
        votes_down: votes.map(|v| v.votes_down()).unwrap_or(0),
        score: votes.map(|v| v.score()).unwrap_or(0.0),
        comments: d.num_comments_public().max(0) as u32,
        tags: d.tags.iter().filter(|t| !t.adminonly()).map(|t| if t.display_name().is_empty() { t.tag() } else { t.display_name() }.to_string()).filter(|t| !t.is_empty()).collect(),
        mature: d.maybe_inappropriate_sex() || d.content_descriptorids.iter().any(|c| MATURE.contains(c)),
        spoiler: d.spoiler_tag(),
    }
}

async fn files(conn: &Connection, q: &FileQuery) -> Result<FilePage, String> {
    let search = q.search.trim();
    let req = CPublishedFileQueryFilesRequest {
        query_type: Some(if search.is_empty() { q.sort } else { 12 }),
        page: Some(q.page.max(1)),
        numperpage: Some(q.per_page.clamp(1, 100)),
        creator_appid: (q.kind == FileKind::Items).then_some(q.app_id),
        appid: Some(q.app_id),
        filetype: Some(q.kind.matching()),
        search_text: Some(search.to_string()).filter(|s| !s.is_empty()),
        days: Some(q.days).filter(|d| *d > 0),
        include_recent_votes_only: Some(q.sort == 3 && q.days > 0),
        requiredtags: q.tags.clone(),
        match_all_tags: Some(true),
        excluded_content_descriptors: if q.mature { vec![] } else { MATURE.to_vec() },
        language: Some(language_id(&q.language)),
        return_previews: Some(true),
        return_short_description: Some(true),
        return_vote_data: Some(true),
        return_tags: Some(true),
        return_details: Some(true),
        ..Default::default()
    };
    let r: CPublishedFileQueryFilesResponse = conn.call("PublishedFile.QueryFiles#1", &req).await.map_err(|e| e.to_string())?;
    let items = r.publishedfiledetails.iter().filter(|d| d.result() == 1 || d.result == None).map(entry).filter(|e| q.mature || !e.mature).collect();
    Ok(FilePage { total: r.total() as u64, items })
}

async fn details(conn: &Connection, ids: Vec<u64>, language: &str, full: bool) -> Result<Vec<PublishedFileDetails>, String> {
    let req = CPublishedFileGetDetailsRequest {
        publishedfileids: ids,
        includetags: Some(true),
        includeadditionalpreviews: Some(full),
        includechildren: Some(full),
        includevotes: Some(true),
        short_description: Some(!full),
        language: Some(language_id(language)),
        ..Default::default()
    };
    let r: CPublishedFileGetDetailsResponse = conn.call("PublishedFile.GetDetails#1", &req).await.map_err(|e| e.to_string())?;
    Ok(r.publishedfiledetails)
}

async fn item(conn: &Connection, me: SteamId, file_id: u64, language: &str) -> Result<Item, String> {
    let d = details(conn, vec![file_id], language, true).await?.into_iter().next().ok_or("Steam didn't find this item")?;
    if d.result() != 1 {
        return Err(match d.result() {
            9 => "This item doesn't exist or was removed.".to_string(),
            15 => "This item is private or friends-only.".to_string(),
            r => format!("Steam couldn't show this item (error {r})"),
        });
    }
    let mut it = Item {
        entry: entry(&d),
        app_name: d.app_name().to_string(),
        description: d.file_description().to_string(),
        lifetime_subscriptions: d.lifetime_subscriptions() as u64,
        file_type: d.file_type(),
        subscribed: d.time_subscribed() > 0,
        banned: d.banned(),
        ..Default::default()
    };
    // The main preview first, then the extra pictures and videos.
    let mut previews: Vec<_> = d.previews.clone();
    previews.sort_by_key(|p| p.sortorder());
    if let Some(p) = &it.entry.preview {
        it.previews.push(Preview { url: Some(p.clone()), youtube: None });
    }
    for p in previews {
        let yt = Some(p.youtubevideoid().to_string()).filter(|s| !s.is_empty());
        let url = Some(p.url().to_string()).filter(|s| !s.is_empty());
        if yt.is_some() || url.is_some() {
            it.previews.push(Preview { url, youtube: yt });
        }
    }
    // Required items: one more lookup for their names.
    let mut children: Vec<_> = d.children.iter().filter(|c| c.file_type() == 0).collect();
    children.sort_by_key(|c| c.sortorder());
    let ids: Vec<u64> = children.iter().map(|c| c.publishedfileid()).collect();
    if !ids.is_empty() {
        if let Ok(list) = details(conn, ids.clone(), language, false).await {
            let by_id: HashMap<u64, FileEntry> = list.iter().filter(|c| c.result() == 1).map(|c| (c.publishedfileid(), entry(c))).collect();
            it.required = ids.iter().filter_map(|i| by_id.get(i).cloned()).collect();
        }
    }
    // Change notes (Workshop items only; guides have none).
    if it.file_type == 0 {
        let req = CPublishedFileGetChangeHistoryRequest { publishedfileid: Some(file_id), total_only: Some(false), startindex: Some(0), count: Some(10), language: Some(language_id(language)) };
        if let Ok(r) = conn.call::<_, CPublishedFileGetChangeHistoryResponse>("PublishedFile.GetChangeHistory#1", &req).await {
            it.change_total = r.total();
            it.change_notes = r.changes.iter().map(|c| ChangeNote { time: c.timestamp() as u64, text: c.change_description().to_string() }).collect();
        }
    }
    if me != 0 {
        if let Ok(r) = conn.call::<_, CPublishedFileGetUserVoteSummaryResponse>("PublishedFile.GetUserVoteSummary#1", &CPublishedFileGetUserVoteSummaryRequest { publishedfileids: vec![file_id] }).await {
            if let Some(s) = r.summaries.first() {
                it.my_vote = if s.vote_for() { Some(true) } else if s.vote_against() { Some(false) } else { None };
            }
        }
    }
    Ok(it)
}

async fn achievements(conn: &Connection, steam_id: SteamId, app_id: AppId, language: &str) -> Result<Vec<model::Achievement>, String> {
    let schema: CPlayerGetGameAchievementsResponse = conn
        .call("Player.GetGameAchievements#1", &CPlayerGetGameAchievementsRequest { appid: Some(app_id), language: Some(language.to_string()), hash_only: Some(false) })
        .await
        .map_err(|e| e.to_string())?;
    let theirs: CPlayerGetUserAchievementsResponse = match conn.call("Player.GetUserAchievements#1", &CPlayerGetUserAchievementsRequest { steamid: Some(steam_id), appid: Some(app_id) }).await {
        Ok(r) => r,
        Err(net::Error::EResult(code, _)) if code == net::eresult::ACCESS_DENIED => return Err("private".into()),
        Err(e) => return Err(e.to_string()),
    };
    if theirs.achievements.is_empty() && !schema.achievements.is_empty() {
        // Steam answers with nothing at all when they don't own the game
        // or hide their game details.
        return Err("none".into());
    }
    let unlocked: HashMap<u32, u64> = theirs.achievements.iter().filter(|a| a.unlocked()).map(|a| (a.internal_key(), a.unlock_time() as u64)).collect();
    let image = |file: &str| if file.is_empty() { String::new() } else { format!("https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/{app_id}/{file}") };
    Ok(schema
        .achievements
        .iter()
        .filter(|a| !a.archived())
        .map(|a| model::Achievement {
            name: a.localized_name().to_string(),
            description: a.localized_desc().to_string(),
            icon: image(a.icon()),
            icon_gray: image(a.icon_gray()),
            hidden: a.hidden(),
            unlocked: unlocked.contains_key(&a.internal_key()),
            unlock_time: unlocked.get(&a.internal_key()).copied().unwrap_or(0),
            global_percent: a.player_percent_unlocked().parse().unwrap_or(0.0),
        })
        .collect())
}

async fn favorite(web: &Web, app_id: AppId, file_id: u64, add: bool) -> Result<(), String> {
    let what = if add { "favorite" } else { "unfavorite" };
    let r = web
        .http
        .post(format!("{COMMUNITY}/sharedfiles/{what}"))
        .header("Referer", format!("{COMMUNITY}/sharedfiles/filedetails/?id={file_id}"))
        .form(&[("id", file_id.to_string()), ("appid", app_id.to_string()), ("sessionid", web.session_id.clone())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() { Ok(()) } else { Err(format!("Steam answered {}", r.status())) }
}

async fn file_comments(web: &Web, file_id: u64, creator: SteamId, start: u32) -> Result<model::Comments, String> {
    let v: serde_json::Value = web
        .http
        .get(format!("{COMMUNITY}/comment/PublishedFile_Public/render/{creator}/{file_id}/"))
        .query(&[("start", start.to_string()), ("count", "10".to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|_| "Comments aren't available".to_string())?;
    if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
        return Err(v.get("error").and_then(|e| e.as_str()).unwrap_or("Comments aren't available").to_string());
    }
    let html = v.get("comments_html").and_then(|h| h.as_str()).unwrap_or_default();
    let total = v.get("total_count").and_then(|n| n.as_u64().or_else(|| n.as_str()?.parse().ok())).unwrap_or(0) as u32;
    Ok(model::Comments { items: crate::social::parse_comments(html), total })
}

async fn news(app_id: AppId, count: u32) -> Result<Vec<News>, String> {
    let v: serde_json::Value = reqwest::Client::new()
        .get("https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/")
        .query(&[("appid", app_id.to_string()), ("count", count.to_string()), ("maxlength", "0".into()), ("format", "json".into())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let items = v.pointer("/appnews/newsitems").and_then(|n| n.as_array()).cloned().unwrap_or_default();
    let s = |i: &serde_json::Value, k: &str| i.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    Ok(items
        .iter()
        .map(|i| News {
            id: s(i, "gid"),
            title: s(i, "title"),
            url: s(i, "url"),
            author: s(i, "author"),
            contents: s(i, "contents").replace("{STEAM_CLAN_IMAGE}", "https://clan.akamai.steamstatic.com/images/"),
            feed: s(i, "feedlabel"),
            time: i.get("date").and_then(|d| d.as_u64()).unwrap_or(0),
            external: i.get("is_external_url").and_then(|d| d.as_bool()).unwrap_or(false),
        })
        .collect())
}

// ---------- HTML ----------

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

/// Decode the HTML entities Steam uses.
pub fn decode(s: &str) -> String {
    let mut out = s.replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    // Numeric entities (&#8217;).
    while let Some(i) = out.find("&#") {
        let Some(end) = out[i..].find(';').map(|e| e + i) else { break };
        let num = &out[i + 2..end];
        let c = if let Some(hex) = num.strip_prefix('x') { u32::from_str_radix(hex, 16).ok() } else { num.parse().ok() }.and_then(char::from_u32);
        match c {
            Some(c) if end - i < 10 => out.replace_range(i..=end, &c.to_string()),
            _ => break,
        }
    }
    out.replace("&amp;", "&")
}

/// Text with tags removed and whitespace tidied.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The inside of the element whose opening tag starts at `at`, matching
/// nested tags of the same name.
fn inner(s: &str, at: usize) -> Option<&str> {
    let rest = &s[at..];
    let name: String = rest.get(1..)?.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let body_start = at + rest.find('>')? + 1;
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let mut depth = 1;
    let mut i = body_start;
    while depth > 0 {
        let next_close = s[i..].find(&close)? + i;
        // An opening tag only counts if the name ends there (`<div` not `<divider`).
        let next_open = s[i..next_close].match_indices(&open).map(|(p, _)| p + i).find(|p| s[p + open.len()..].starts_with([' ', '>', '\n', '\t', '/']));
        match next_open {
            Some(o) => {
                depth += 1;
                i = o + open.len();
            }
            None => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[body_start..next_close]);
                }
                i = next_close + close.len();
            }
        }
    }
    None
}

/// The inside of the first element with this exact opening text
/// (e.g. `<div class="subSectionDesc"`).
fn element<'a>(s: &'a str, open: &str) -> Option<&'a str> {
    inner(s, s.find(open)?)
}

pub fn parse_guide(body: &str) -> Result<Guide, String> {
    if !body.contains("class=\"guideTop\"") && !body.contains("guide subSections") {
        if body.contains("error_ctn") || body.contains("There was a problem accessing the item") {
            return Err("This guide is private, friends-only or was removed.".into());
        }
        return Err("Steam didn't send the guide".into());
    }
    let text = |open: &str| element(body, open).map(|h| decode(h.trim())).unwrap_or_default();
    let mut sections = Vec::new();
    if let Some(list) = element(body, "<div class=\"guide subSections\"") {
        let mut rest = list;
        while let Some(at) = rest.find("<div class=\"subSection detailBox\"") {
            let block = inner(rest, at).unwrap_or_default();
            let title = element(block, "<div class=\"subSectionTitle\"").map(strip_tags).unwrap_or_default();
            let html = element(block, "<div class=\"subSectionDesc\"").unwrap_or_default().trim().to_string();
            sections.push(Section { title, html });
            rest = &rest[at + 10..];
        }
    }
    Ok(Guide {
        title: strip_tags(&text("<div class=\"workshopItemTitle\"")),
        authors: strip_tags(&text("<div class=\"guideAuthors\"")).trim_start_matches("By ").to_string(),
        overview: element(body, "<div class=\"guideTopDescription\"").unwrap_or_default().trim().to_string(),
        sections,
    })
}

/// The number in Steam's "Showing 1-15 of 247" paging line.
fn paging_total(s: &str) -> Option<u64> {
    let span = between(s, "_pagetotal\">", "<")?;
    span.trim().replace([',', '.', ' '], "").parse().ok()
}

pub fn parse_topics(body: &str, app_id: AppId) -> Result<TopicPage, String> {
    if !body.contains("forum_topics") && !body.contains("forum_list") {
        if body.contains("is private") || body.contains("must be logged in") {
            return Err("This forum is private.".into());
        }
        return Err("This game has no discussions.".into());
    }
    let base = format!("{COMMUNITY}/app/{app_id}/");
    let mut forums = Vec::new();
    if let Some(list) = element(body, "<div class=\"rightbox forum_list\"") {
        for block in list.split("<div class=\"rightbox_list_option").skip(1) {
            let threads = between(block, "<div class=\"forum_list_postcount\">", "</div>").map(|n| n.trim().replace([',', '.', ' ', '\u{a0}'], "")).and_then(|n| n.parse().ok()).unwrap_or(0);
            let Some(href) = between(block, "href=\"", "\"") else { continue };
            let Some(path) = href.strip_prefix(&base) else { continue };
            let name = between(block, "\">", "</a>").map(strip_tags).unwrap_or_default();
            let name = block.find("<a ").and_then(|i| between(&block[i..], ">", "</a>")).map(strip_tags).unwrap_or(name);
            forums.push(Forum { name, path: path.to_string(), threads });
        }
    }
    let forum = between(body, "\"forum_display_name\":\"", "\"").map(|s| decode(&s.replace("\\/", "/"))).unwrap_or_else(|| {
        // The forum page's breadcrumb as a fallback.
        between(body, "<div class=\"forum_breadcrumbs\">", "</div>").map(strip_tags).unwrap_or_default()
    });
    let mut topics = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("class=\"forum_topic ") {
        let start = rest[..at].rfind("<div").unwrap_or(at);
        let open_end = rest[at..].find('>').map(|e| e + at).unwrap_or(at);
        let open = &rest[start..open_end];
        let block = inner(rest, start).unwrap_or_default();
        rest = &rest[open_end..];
        if open.contains("forum_topic_mini") {
            continue;
        }
        let classes = between(open, "class=\"", "\"").unwrap_or_default();
        let Some(id) = between(open, "data-gidforumtopic=\"", "\"").and_then(|i| i.parse().ok()) else { continue };
        let url = between(block, "class=\"forum_topic_overlay\" href=\"", "\"").unwrap_or_default().to_string();
        let name_html = element(block, "<div class=\"forum_topic_name").unwrap_or_default();
        let name_html = match name_html.find("</span>") {
            Some(i) if name_html.contains("forum_topic_label") => &name_html[i + 7..],
            _ => name_html,
        };
        let tooltip = between(open, "data-tooltip-forum=\"", "\"").map(decode).unwrap_or_default();
        let preview = element(&tooltip, "<div class=\"topic_hover_text\"").map(strip_tags).unwrap_or_default();
        topics.push(Topic {
            id,
            title: strip_tags(name_html),
            author: element(block, "<div class=\"forum_topic_op\"").map(strip_tags).unwrap_or_default(),
            replies: element(block, "<div class=\"forum_topic_reply_count\"").map(strip_tags).and_then(|n| n.replace([',', '.', ' '], "").parse().ok()).unwrap_or(0),
            last_post: between(block, "data-timestamp=\"", "\"").and_then(|t| t.parse().ok()).unwrap_or(0),
            pinned: classes.split_whitespace().any(|c| c == "sticky"),
            locked: classes.split_whitespace().any(|c| c == "locked"),
            preview,
            url,
        });
    }
    let total = paging_total(body).unwrap_or(topics.len() as u64);
    Ok(TopicPage { forum, total, page_size: 15, topics, forums })
}

fn post(block: &str, author_class: &str, content_open: &str) -> Post {
    let author_at = block.find(author_class).unwrap_or(0);
    let author_block = &block[author_at..];
    let account: u64 = between(author_block, "data-miniprofile=\"", "\"").and_then(|a| a.parse().ok()).unwrap_or(0);
    let name = between(author_block, ">", "</a>").map(strip_tags).unwrap_or_default();
    // The avatar is the image inside the profile link (after any frame).
    let avatar = block.find("Avatar").and_then(|i| {
        let rest = &block[i..];
        let link = rest.find("<a href=")?;
        let img = between(&rest[link..], "<img src=\"", "\"")?;
        img.starts_with("http").then(|| img.to_string())
    });
    Post {
        id: 0,
        author: if account > 0 { account + BASE_ID } else { 0 },
        name,
        avatar,
        time: between(block, "data-timestamp=\"", "\"").and_then(|t| t.parse().ok()).unwrap_or(0),
        html: element(block, content_open).unwrap_or_default().trim().to_string(),
    }
}

pub fn parse_thread(body: &str) -> Result<Thread, String> {
    let Some(op_at) = body.find("class=\"forum_op ") else {
        if body.contains("is private") || body.contains("not available") {
            return Err("This discussion is private or was removed.".into());
        }
        return Err("Steam didn't send the discussion".into());
    };
    let op_start = body[..op_at].rfind("<div").unwrap_or(op_at);
    let op_block = inner(body, op_start).unwrap_or_default();
    let mut op = post(op_block, "forum_op_author", "<div class=\"content\"");
    op.id = between(op_block, "id=\"forum_op_", "\"").and_then(|i| i.parse().ok()).unwrap_or(0);
    let title = element(op_block, "<div class=\"topic\"").map(strip_tags).unwrap_or_default();
    let json = between(body, "InitializeCommentThread(", "</script>").unwrap_or_default();
    let total = between(json, "\"total_count\":", ",").and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    let page_size = between(json, "\"pagesize\":", ",").and_then(|n| n.trim().parse().ok()).unwrap_or(15);
    let forum = between(body, "\"forum_display_name\":\"", "\"").map(|s| decode(&s.replace("\\/", "/"))).unwrap_or_default();
    let forum_url = between(body, "\"forum_url\":\"", "\"").map(|s| s.replace("\\/", "/")).unwrap_or_default();
    let can_reply = between(body, "\"can_reply\":", ",").map(|v| v.trim() == "1").unwrap_or(true);
    let mut posts = Vec::new();
    if let Some(list) = body.find("class=\"commentthread_comments\"").map(|i| &body[i..]) {
        let mut rest = list;
        while let Some(at) = rest.find("class=\"commentthread_comment ") {
            let start = rest[..at].rfind("<div").unwrap_or(at);
            let block = inner(rest, start).unwrap_or_default();
            let mut p = post(block, "commentthread_author_link", "<div class=\"commentthread_comment_text\"");
            p.id = between(&rest[start..], "id=\"comment_", "\"").and_then(|i| i.parse().ok()).unwrap_or(0);
            posts.push(p);
            rest = &rest[at + 10..];
        }
    }
    Ok(Thread { title, forum, forum_url, op, posts, total, page_size, locked: !can_reply || op_block.contains("forum_icon_Locked") })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPICS: &str = r#"
<div class="rightbox forum_list"><div class="content">
  <div class="rightbox_list_header"><div class="forum_list_postcount">THREADS</div><div class="forum_list_name">SUB FORUMS</div></div>
  <div class="rightbox_list_option selected "><div class="rightbox_list_option_content">
    <div class="forum_list_postcount">50,411</div>
    <div class="forum_list_name"><a class="whiteLink" href="https://steamcommunity.com/app/413150/discussions/0/">General Discussions</a></div>
  </div></div>
  <div class="rightbox_list_option "><div class="rightbox_list_option_content">
    <div class="forum_list_postcount">1,203</div>
    <div class="forum_list_name"><a class="whiteLink" href="https://steamcommunity.com/app/413150/tradingforum/">Trading</a></div>
  </div></div>
</div></div>
<script>InitializeForum({"forum_display_name":"General Discussions"})</script>
<div class="forum_paging_summary ellipsis">Showing <span id="x_pagestart">1</span>-<span id="x_pageend">15</span> of <span id="x_pagetotal">247</span> active topics</div>
<div id="forum_General_topics" class="forum_topics">
<div role="button" class="forum_topic  unread locked sticky" id="forum_General_1"  data-gidforumtopic="582806854239890446" data-tooltip-forum="&lt;div class=&quot;topic_hover_text&quot;&gt; Welcome to Stardew Valley! &lt;/div&gt;">
  <a class="forum_topic_overlay" href="https://steamcommunity.com/app/413150/discussions/0/582806854239890446/"></a>
  <div class="forum_topic_details "><div class="forum_topic_reply_count"><img src="x.png"> 0</div>
    <div class="forum_topic_lastpost" title="t" data-timestamp="1788104417">Aug 30</div></div>
  <div class="forum_topic_icon"><img src="lock.png"></div>
  <div class="forum_topic_name "><span class="forum_topic_label sticky_label">PINNED:</span> Community Guidelines</div>
  <div class="forum_topic_op">ChrillBill</div>
</div>
<div role="button" class="forum_topic  unread" id="forum_General_2" data-gidforumtopic="564798618066955213" data-tooltip-forum="">
  <a class="forum_topic_overlay" href="https://steamcommunity.com/app/413150/discussions/0/564798618066955213/"></a>
  <div class="forum_topic_details "><div class="forum_topic_reply_count"><img src="x.png"> 2</div>
    <div class="forum_topic_lastpost" data-timestamp="1794078381">49 minutes ago</div></div>
  <div class="forum_topic_name ">how do i delete old tools?</div>
  <div class="forum_topic_op">someone</div>
</div>
</div>"#;

    #[test]
    fn reads_forum_pages() {
        let p = parse_topics(TOPICS, 413150).unwrap();
        assert_eq!(p.total, 247);
        assert_eq!(p.forum, "General Discussions");
        assert_eq!(p.forums, vec![
            Forum { name: "General Discussions".into(), path: "discussions/0/".into(), threads: 50411 },
            Forum { name: "Trading".into(), path: "tradingforum/".into(), threads: 1203 },
        ]);
        assert_eq!(p.topics.len(), 2);
        let t = &p.topics[0];
        assert_eq!((t.id, t.title.as_str(), t.author.as_str(), t.replies), (582806854239890446, "Community Guidelines", "ChrillBill", 0));
        assert!(t.pinned && t.locked);
        assert_eq!(t.preview, "Welcome to Stardew Valley!");
        assert_eq!(p.topics[1].replies, 2);
        assert_eq!(p.topics[1].last_post, 1794078381);
        assert!(p.topics[1].url.ends_with("/564798618066955213/"));
    }

    const THREAD: &str = r#"
<div data-panel="x" class="forum_op " id="forum_op_564798618066955213">
  <div class="forum_op_header commentthread_comment_author">
    <div class="forum_op_avatar playerAvatar online">
      <div class="profile_avatar_frame"><img src="https://frame.png"></div>
      <a href="https://steamcommunity.com/profiles/76561197960265731"><img src="https://avatars.example/op.jpg" data-miniprofile="3"></a>
    </div>
    <div class="authorline"><a class="hoverunderline forum_op_author " href="x" data-miniprofile="3">Asker<span class="forum_author_action_pulldown"></span></a>
    <div class="date commentthread_comment_timestamp" data-timestamp="1794053854">7 hours ago</div></div>
  </div>
  <div class="topic" id="forum_op_topic_564798618066955213">how do i delete old tools?</div>
  <div class="content" id="forum_op_content_564798618066955213">I have <b>too many</b><div class="bb_h1">tools</div> help</div>
  <div class="forum_op_footer"><div></div></div>
</div>
<script>InitializeForumTopic( {"forum_display_name":"General Discussions","forum_url":"https:\/\/steamcommunity.com\/app\/413150\/discussions\/0\/","permissions":{"can_reply":1,"can_post":1}});</script>
<script>$J( function() { InitializeCommentThread( "ForumTopic", "x", {"total_count":2,"start":0,"pagesize":15,"has_upvoted":0}); } );</script>
<div class="commentthread_comments" id="x_posts">
  <div data-panel="y" class="commentthread_comment responsive_body_text   " id="comment_564798618066956854" style="">
    <div class="commentthread_comment_content"><div class="commentthread_comment_author">
      <div class="commentthread_comment_avatar playerAvatar offline"><a href="https://steamcommunity.com/profiles/76561197960265732" data-miniprofile="4"><img src="https://avatars.example/a.jpg" srcset="x"></a></div>
      <div class="author_name_group"><a class="hoverunderline commentthread_author_link " href="x" data-miniprofile="4"><bdi>Aluvard<span class="forum_author_action_pulldown"></span></bdi></a>
      <div class="commentthread_comment_timestamp" data-timestamp="1794055275">7 hours ago</div></div></div>
      <div class="commentthread_comment_text" id="comment_content_564798618066956854">Put the old one in a <div class="bb_blockquote">quote</div> chest.</div>
    </div>
  </div>
  <div data-panel="y" class="commentthread_comment responsive_body_text   " id="comment_2">
    <div class="commentthread_comment_author"><a class="hoverunderline commentthread_author_link " data-miniprofile="1"><bdi>Two</bdi></a></div>
    <div class="commentthread_comment_text" id="comment_content_2">Second</div>
  </div>
</div>"#;

    #[test]
    fn reads_threads() {
        let t = parse_thread(THREAD).unwrap();
        assert_eq!(t.title, "how do i delete old tools?");
        assert_eq!(t.forum, "General Discussions");
        assert_eq!(t.forum_url, "https://steamcommunity.com/app/413150/discussions/0/");
        assert_eq!((t.total, t.page_size), (2, 15));
        assert_eq!(t.op.name, "Asker");
        assert_eq!(t.op.author, 76561197960265731);
        assert_eq!(t.op.avatar.as_deref(), Some("https://avatars.example/op.jpg"));
        assert_eq!(t.op.html, "I have <b>too many</b><div class=\"bb_h1\">tools</div> help");
        assert_eq!(t.posts.len(), 2);
        assert_eq!(t.posts[0].name, "Aluvard");
        assert_eq!(t.posts[0].id, 564798618066956854);
        assert_eq!(t.posts[0].time, 1794055275);
        assert!(t.posts[0].html.ends_with("chest."), "{}", t.posts[0].html);
        assert_eq!(t.posts[1].html, "Second");
        assert!(!t.locked);
    }

    #[test]
    fn reads_guides() {
        let page = r#"<div class="guideTop"><div class="workshopItemTitle">How to farm</div><div class="guideAuthors">By Ferris</div>
<div class="guideTopDescription" id="highlightContent">The basics &amp; more</div></div>
<div class="guide subSections">
  <div class="subSection detailBox" id="1"><div class="subSectionTitle">Getting started	</div><div class="subSectionDesc">Plant <b>seeds</b>.<div class="bb_h2">Water</div></div></div>
  <div style="clear: both"></div>
  <div class="subSection detailBox" id="2"><div class="subSectionTitle">Selling</div><div class="subSectionDesc"><img src="https://img/1.png"></div></div>
</div>"#;
        let g = parse_guide(page).unwrap();
        assert_eq!(g.title, "How to farm");
        assert_eq!(g.authors, "Ferris");
        assert_eq!(g.overview, "The basics &amp; more");
        assert_eq!(g.sections.len(), 2);
        assert_eq!(g.sections[0].title, "Getting started");
        assert_eq!(g.sections[0].html, "Plant <b>seeds</b>.<div class=\"bb_h2\">Water</div>");
        assert_eq!(g.sections[1].html, "<img src=\"https://img/1.png\">");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode("Tom&#39;s &amp; Jerry&#8217;s &lt;3"), "Tom's & Jerry\u{2019}s <3");
        assert_eq!(strip_tags("<b>a</b>   b\n<br>c"), "a b c");
    }

    #[test]
    fn star_rating() {
        assert_eq!(stars(1, 0), None);
        assert_eq!(stars(100, 0), Some(5));
        assert_eq!(stars(50, 50), Some(3));
        assert_eq!(stars(1, 99), Some(1));
    }
}

/// `HUB_HTML=<folder with forums.html, thread.html, guide.html> cargo test -p client real_pages -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn real_pages() {
    let dir = std::env::var("HUB_HTML").expect("HUB_HTML");
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).unwrap();
    let p = parse_topics(&read("forums.html"), 413150).unwrap();
    println!("forum {:?} total {} forums {:?}", p.forum, p.total, p.forums);
    for t in p.topics.iter().take(4) {
        println!("  {:?} by {} replies {} pinned {} locked {} at {} | {:.60}", t.title, t.author, t.replies, t.pinned, t.locked, t.last_post, t.preview);
    }
    let t = parse_thread(&read("thread.html")).unwrap();
    println!("thread {:?} in {:?} total {} op {:?} {:?} posts {}", t.title, t.forum, t.total, t.op.name, t.op.avatar, t.posts.len());
    for p in &t.posts {
        println!("  {} {} {:?} {:?}", p.id, p.name, p.avatar, p.html);
    }
    let g = parse_guide(&read("guide.html")).unwrap();
    println!("guide {:?} by {:?} sections {}", g.title, g.authors, g.sections.len());
    for s in g.sections.iter().take(3) {
        println!("  {:?}: {:.100}", s.title, s.html);
    }
}

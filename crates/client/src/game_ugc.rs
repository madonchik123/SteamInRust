//! Workshop for games (ISteamUGC in our steamclient): queries, the player's
//! subscriptions, votes and item downloads, through Steam's PublishedFile
//! service. Downloads go through [`crate::workshop`], like the library's.

use crate::ipc::protocol::{Reply, UgcItem, UgcQuery};
use model::AppId;
use net::{Connection, eresult};
use proto::*;
use std::path::Path;

/// Steam's page size for UGC queries (kNumUGCResultsPerPage).
const PER_PAGE: u32 = 50;

fn code(e: &net::Error) -> i32 {
    match e {
        net::Error::EResult(c, _) => *c,
        _ => eresult::FAIL,
    }
}

fn steam_id_of(account: u32) -> u64 {
    0x0110_0001_0000_0000 | account as u64
}

/// EUGCMatchingUGCType → EPublishedFileInfoMatchingFileType.
fn file_type(matching: i32) -> u32 {
    match matching {
        1 => 17,  // Items_Mtx
        2 => 18,  // Items_ReadyToUse
        3 => 1,   // Collections
        4 => 2,   // Artwork
        5 => 3,   // Videos
        6 => 4,   // Screenshots
        7 => 10,  // AllGuides
        8 => 11,  // WebGuides
        9 => 12,  // IntegratedGuides
        10 => 13, // UsableInGame
        11 => 15, // ControllerBindings
        12 => 20, // GameManagedItems
        _ => 0,   // Items (and "all")
    }
}

/// EUserUGCList → GetUserFiles' list name.
fn user_list(list: i32) -> &'static str {
    match list {
        1 => "votedon",
        2 => "votedup",
        3 => "voteddown",
        4 => "votelater",
        5 => "myfavorites",
        6 => "mysubscriptions",
        7 => "usedorplayed",
        8 => "followed",
        _ => "myfiles",
    }
}

/// EUserUGCListSortOrder → GetUserFiles' sort name.
fn user_sort(sort: i32) -> &'static str {
    match sort {
        1 => "createdasc",
        2 => "title",
        3 => "lastupdated",
        4 => "subscriptiondate",
        5 => "score",
        6 => "formoderation",
        _ => "created",
    }
}

pub fn item(d: &PublishedFileDetails) -> UgcItem {
    let votes = d.vote_data.as_ref();
    let mut stats = vec![
        (0, d.subscriptions() as u64),
        (1, d.favorited() as u64),
        (2, d.followers() as u64),
        (3, d.lifetime_subscriptions() as u64),
        (4, d.lifetime_favorited() as u64),
        (5, d.lifetime_followers() as u64),
        (6, d.views() as u64),
        (8, d.lifetime_playtime()),
        (9, d.lifetime_playtime_sessions()),
        (10, d.num_comments_public().max(0) as u64),
    ];
    if let Some(p) = &d.playtime_stats {
        stats.push((11, p.playtime_seconds()));
        stats.push((12, p.num_sessions()));
    }
    UgcItem {
        id: d.publishedfileid(),
        result: d.result() as i32,
        file_type: d.file_type() as i32,
        creator_app: d.creator_appid(),
        consumer_app: d.consumer_appid(),
        title: d.title().to_string(),
        description: if d.file_description().is_empty() { d.short_description().to_string() } else { d.file_description().to_string() },
        owner: d.creator(),
        created: d.time_created(),
        updated: d.time_updated(),
        visibility: d.visibility() as i32,
        banned: d.banned(),
        accepted: d.workshop_accepted(),
        tags: d.tags.iter().filter(|t| !t.adminonly()).map(|t| (t.tag().to_string(), t.display_name().to_string())).collect(),
        file: d.hcontent_file(),
        preview_file: d.hcontent_preview(),
        file_name: d.filename().to_string(),
        file_size: d.file_size(),
        preview_size: d.preview_file_size(),
        url: d.url().to_string(),
        votes_up: votes.map(|v| v.votes_up()).unwrap_or(0),
        votes_down: votes.map(|v| v.votes_down()).unwrap_or(0),
        score: votes.map(|v| v.score()).unwrap_or(0.0),
        children: d.children.iter().map(|c| c.publishedfileid()).collect(),
        preview_url: d.preview_url().to_string(),
        metadata: d.metadata().to_string(),
        kv: d.kvtags.iter().map(|t| (t.key().to_string(), t.value().to_string())).collect(),
        previews: d
            .previews
            .iter()
            .map(|p| if p.youtubevideoid().is_empty() { (p.url().to_string(), p.preview_type() as i32, p.filename().to_string()) } else { (p.youtubevideoid().to_string(), 1, String::new()) })
            .collect(),
        stats,
    }
}

pub async fn query(conn: &Connection, app_id: AppId, q: &UgcQuery) -> Reply {
    let language = crate::hub::language_id(if q.language.is_empty() { "english" } else { &q.language });
    let consumer = if q.consumer_app == 0 { app_id } else { q.consumer_app };
    let creator = if q.creator_app == 0 { consumer } else { q.creator_app };
    let fail = |e: net::Error| {
        tracing::info!("workshop query failed: {e}");
        Reply::UgcResults { eresult: code(&e), total: 0, items: Vec::new(), cursor: String::new() }
    };
    let ok = |total: u32, details: &[PublishedFileDetails], cursor: String| Reply::UgcResults { eresult: eresult::OK, total, items: details.iter().map(item).collect(), cursor };
    match q.kind {
        2 => {
            let req = CPublishedFileGetDetailsRequest {
                publishedfileids: q.ids.clone(),
                includetags: Some(true),
                includeadditionalpreviews: Some(q.previews),
                includechildren: Some(q.children),
                includekvtags: Some(q.kv_tags),
                includevotes: Some(true),
                short_description: Some(!q.long_description),
                includemetadata: Some(q.metadata),
                language: Some(language),
                return_playtime_stats: Some(q.playtime_days).filter(|d| *d > 0),
                appid: Some(consumer),
                ..Default::default()
            };
            match conn.call::<_, CPublishedFileGetDetailsResponse>("PublishedFile.GetDetails#1", &req).await {
                Ok(r) => ok(r.publishedfiledetails.len() as u32, &r.publishedfiledetails, String::new()),
                Err(e) => fail(e),
            }
        }
        0 => {
            let req = CPublishedFileGetUserFilesRequest {
                steamid: Some(steam_id_of(q.account)),
                appid: Some(consumer),
                creator_appid: Some(creator),
                page: Some(q.page.max(1)),
                numperpage: Some(PER_PAGE),
                r#type: Some(user_list(q.list).into()),
                sortmethod: Some(user_sort(q.sort).into()),
                filetype: Some(file_type(q.matching)),
                requiredtags: q.required_tags.clone(),
                excludedtags: q.excluded_tags.clone(),
                match_cloud_filename: Some(q.cloud_filename.clone()).filter(|s| !s.is_empty()),
                cache_max_age_seconds: Some(q.max_age),
                language: Some(language),
                totalonly: Some(q.total_only),
                ids_only: Some(q.only_ids),
                return_vote_data: Some(true),
                return_tags: Some(true),
                return_kv_tags: Some(q.kv_tags),
                return_previews: Some(q.previews),
                return_children: Some(q.children),
                return_short_description: Some(!q.long_description),
                return_metadata: Some(q.metadata),
                return_playtime_stats: Some(q.playtime_days).filter(|d| *d > 0),
                ..Default::default()
            };
            match conn.call::<_, CPublishedFileGetUserFilesResponse>("PublishedFile.GetUserFiles#1", &req).await {
                Ok(r) => ok(r.total(), &r.publishedfiledetails, String::new()),
                Err(e) => fail(e),
            }
        }
        _ => {
            let req = CPublishedFileQueryFilesRequest {
                query_type: Some(q.query.max(0) as u32),
                page: if q.cursor.is_some() { None } else { Some(q.page.max(1)) },
                cursor: q.cursor.clone(),
                numperpage: Some(PER_PAGE),
                creator_appid: Some(creator),
                appid: Some(consumer),
                requiredtags: q.required_tags.clone(),
                excludedtags: q.excluded_tags.clone(),
                match_all_tags: Some(!q.match_any),
                search_text: Some(q.search.clone()).filter(|s| !s.is_empty()),
                filetype: Some(file_type(q.matching)),
                days: Some(q.trend_days).filter(|d| *d > 0),
                cache_max_age_seconds: Some(q.max_age),
                language: Some(language),
                required_kv_tags: q.kv.iter().map(|(k, v)| c_published_file_query_files_request::KvTag { key: Some(k.clone()), value: Some(v.clone()) }).collect(),
                taggroups: q.tag_groups.iter().map(|g| c_published_file_query_files_request::TagGroup { tags: g.clone() }).collect(),
                date_range_created: (q.created != (0, 0))
                    .then_some(c_published_file_query_files_request::DateRange { timestamp_start: Some(q.created.0), timestamp_end: Some(q.created.1) }),
                date_range_updated: (q.updated != (0, 0))
                    .then_some(c_published_file_query_files_request::DateRange { timestamp_start: Some(q.updated.0), timestamp_end: Some(q.updated.1) }),
                totalonly: Some(q.total_only),
                ids_only: Some(q.only_ids),
                return_vote_data: Some(true),
                return_tags: Some(true),
                return_kv_tags: Some(q.kv_tags),
                return_previews: Some(q.previews),
                return_children: Some(q.children),
                return_short_description: Some(!q.long_description),
                return_metadata: Some(q.metadata),
                return_playtime_stats: Some(q.playtime_days).filter(|d| *d > 0),
                return_details: Some(true),
                ..Default::default()
            };
            match conn.call::<_, CPublishedFileQueryFilesResponse>("PublishedFile.QueryFiles#1", &req).await {
                Ok(r) => ok(r.total(), &r.publishedfiledetails, r.next_cursor().to_string()),
                Err(e) => fail(e),
            }
        }
    }
}

/// Every item the player subscribed to for this game.
pub async fn subscribed(conn: &Connection, app_id: AppId, me: u64) -> Vec<u64> {
    crate::workshop::subscribed(conn, me, app_id).await.unwrap_or_else(|e| {
        tracing::info!("workshop subscriptions: {e}");
        Vec::new()
    })
}

pub async fn subscribe(conn: &Connection, app_id: AppId, id: u64, list: u32, on: bool) -> i32 {
    let r = if on {
        conn.call::<_, CPublishedFileSubscribeResponse>(
            "PublishedFile.Subscribe#1",
            &CPublishedFileSubscribeRequest { publishedfileid: Some(id), list_type: Some(list), appid: Some(app_id as i32), notify_client: Some(true), include_dependencies: Some(false) },
        )
        .await
        .map(|_| ())
    } else {
        conn.call::<_, CPublishedFileUnsubscribeResponse>(
            "PublishedFile.Unsubscribe#1",
            &CPublishedFileUnsubscribeRequest { publishedfileid: Some(id), list_type: Some(list), appid: Some(app_id as i32), notify_client: Some(true) },
        )
        .await
        .map(|_| ())
    };
    r.map(|_| eresult::OK).unwrap_or_else(|e| code(&e))
}

pub async fn vote(conn: &Connection, id: u64, up: bool) -> i32 {
    conn.call::<_, CPublishedFileVoteResponse>("PublishedFile.Vote#1", &CPublishedFileVoteRequest { publishedfileid: Some(id), vote_up: Some(up) })
        .await
        .map(|_| eresult::OK)
        .unwrap_or_else(|e| code(&e))
}

pub async fn get_vote(conn: &Connection, id: u64) -> Reply {
    match conn.call::<_, CPublishedFileGetUserVoteSummaryResponse>("PublishedFile.GetUserVoteSummary#1", &CPublishedFileGetUserVoteSummaryRequest { publishedfileids: vec![id] }).await {
        Ok(r) => {
            let s = r.summaries.iter().find(|s| s.publishedfileid() == id);
            Reply::UgcVote { eresult: eresult::OK, up: s.is_some_and(|s| s.vote_for()), down: s.is_some_and(|s| s.vote_against()), skipped: s.is_some_and(|s| s.vote_later()) }
        }
        Err(e) => Reply::UgcVote { eresult: code(&e), up: false, down: false, skipped: false },
    }
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()).unwrap_or(0)
}

/// Download (or update) one item for the game, through the same path (and
/// `appworkshop_<app>.acf` record) as the library's Workshop downloads.
pub async fn download(conn: &Connection, app_id: AppId, me: u64, id: u64, install_dir: &Path, cell_id: u32) -> Reply {
    let failed = |why: String| {
        tracing::info!("workshop item {id}: {why}");
        Reply::UgcDownloaded { eresult: eresult::FAIL, folder: String::new(), size: 0, updated: 0 }
    };
    let Some(library) = crate::workshop::library_of(install_dir) else { return failed("the game isn't in a Steam library folder".into()) };
    match crate::workshop::sync(conn, me, app_id, &library, cell_id, vec![id]).await {
        Ok(r) if r.failed.is_empty() => {
            let dir = library.join("steamapps").join("workshop").join("content").join(app_id.to_string()).join(id.to_string());
            if !dir.is_dir() {
                return failed("not available".into());
            }
            let updated = std::fs::metadata(&dir).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as u32).unwrap_or(0);
            Reply::UgcDownloaded { eresult: eresult::OK, folder: dir.to_string_lossy().into_owned(), size: dir_size(&dir), updated }
        }
        Ok(r) => failed(r.failed.iter().map(|f| f.2.clone()).collect::<Vec<_>>().join("; ")),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_steamworks_enums() {
        assert_eq!(file_type(0), 0);
        assert_eq!(file_type(3), 1);
        assert_eq!(file_type(-1), 0);
        assert_eq!(user_list(6), "mysubscriptions");
        assert_eq!(steam_id_of(1), 76561197960265729);
        let d = PublishedFileDetails { publishedfileid: Some(5), title: Some("Map".into()), short_description: Some("short".into()), ..Default::default() };
        let i = item(&d);
        assert_eq!((i.id, i.title.as_str(), i.description.as_str()), (5, "Map", "short"));
    }
}

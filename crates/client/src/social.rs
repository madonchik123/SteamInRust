//! Profile comments and badges from steamcommunity.com (no API for these).

use crate::market::Web;
use model::SteamId;
use serde_json::Value;

const COMMUNITY: &str = "https://steamcommunity.com";
const BASE_ID: u64 = 76561197960265728;

fn http(web: Option<&Web>) -> reqwest::Client {
    web.map(|w| w.http.clone()).unwrap_or_default()
}

/// A page of comments, newest first. `Err` carries Steam's reason, e.g.
/// "This profile is private." or comments being turned off.
pub async fn comments(web: Option<&Web>, steam_id: SteamId, start: u32) -> Result<model::Comments, String> {
    let v: Value = http(web)
        .get(format!("{COMMUNITY}/comment/Profile/render/{steam_id}/-1/"))
        .query(&[("start", start.to_string()), ("count", "20".to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|_| "Comments aren't available".to_string())?;
    page(&v)
}

fn page(v: &Value) -> Result<model::Comments, String> {
    if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
        return Err(v.get("error").and_then(|e| e.as_str()).unwrap_or("Comments aren't available").to_string());
    }
    let html = v.get("comments_html").and_then(|h| h.as_str()).unwrap_or_default();
    let total = v.get("total_count").and_then(|n| n.as_u64().or_else(|| n.as_str()?.parse().ok())).unwrap_or(0) as u32;
    Ok(model::Comments { items: parse_comments(html), total })
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

pub fn parse_comments(html: &str) -> Vec<model::Comment> {
    html.split("class=\"commentthread_comment ")
        .skip(1)
        .filter_map(|block| {
            let id = between(block, "id=\"comment_", "\"")?.parse().ok()?;
            // The author's link carries their 32-bit account id.
            let author_block = &block[block.find("commentthread_author_link")?..];
            let account: u64 = between(author_block, "data-miniprofile=\"", "\"")?.parse().ok()?;
            let name = between(author_block, "<bdi>", "</bdi>").unwrap_or_default().trim().to_string();
            // Avatar: first image inside the avatar link (after any frame).
            let avatar = block.find("<a href=").and_then(|i| {
                let rest = &block[i..];
                let url_at = ["srcset=\"", "src=\""].iter().filter_map(|k| rest.find(k).map(|p| p + k.len())).min()?;
                let url: String = rest[url_at..].chars().take_while(|c| *c != '"' && *c != ' ').collect();
                url.starts_with("http").then_some(url)
            });
            let time = between(block, "data-timestamp=\"", "\"").and_then(|t| t.parse().ok()).unwrap_or(0);
            let text_start = block.find("class=\"commentthread_comment_text\"")?;
            let text = between(&block[text_start..], ">", "</div>").unwrap_or_default().trim().to_string();
            Some(model::Comment { id, author: account + BASE_ID, name: unescape(&name), avatar, time, html: text })
        })
        .collect()
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// Leave a comment. Needs the signed-in web session.
pub async fn post_comment(web: &Web, steam_id: SteamId, text: &str) -> Result<model::Comments, String> {
    let v: Value = web
        .http
        .post(format!("{COMMUNITY}/comment/Profile/post/{steam_id}/-1/"))
        .header("Referer", format!("{COMMUNITY}/profiles/{steam_id}/"))
        .form(&[("comment", text), ("count", "20"), ("sessionid", web.session_id.as_str()), ("feature2", "-1")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    page(&v)
}

/// Block (or unblock) all communication with someone, like the profile's
/// "Block all communication".
pub async fn block_user(web: &Web, steam_id: SteamId, block: bool) -> Result<bool, String> {
    let resp = web
        .http
        .post(format!("{COMMUNITY}/actions/BlockUserAjax"))
        .header("Referer", format!("{COMMUNITY}/profiles/{steam_id}/"))
        .form(&[("sessionid", web.session_id.clone()), ("steamid", steam_id.to_string()), ("block", if block { "1" } else { "0" }.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(block) } else { Err(format!("Steam said {}", resp.status())) }
}

/// Delete a comment. Steam allows it on your own profile and for your own comments.
pub async fn delete_comment(web: &Web, steam_id: SteamId, comment_id: u64) -> Result<model::Comments, String> {
    let v: Value = web
        .http
        .post(format!("{COMMUNITY}/comment/Profile/delete/{steam_id}/-1/"))
        .header("Referer", format!("{COMMUNITY}/profiles/{steam_id}/"))
        .form(&[("gidcomment", comment_id.to_string()), ("start", "0".into()), ("count", "20".into()), ("sessionid", web.session_id.clone()), ("feature2", "-1".into())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    page(&v)
}

/// Level, XP and badges from the profile's badges page.
pub async fn badges(web: Option<&Web>, steam_id: SteamId) -> Result<model::Badges, String> {
    let body = http(web).get(format!("{COMMUNITY}/profiles/{steam_id}/badges/")).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    if !body.contains("badge_row") && body.contains("profile_private_info") {
        return Err("private".into());
    }
    Ok(parse_badges(&body))
}

fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    unescape(&out).replace("&nbsp;", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_badges(body: &str) -> model::Badges {
    let level = between(body, "friendPlayerLevelNum\">", "<").and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    let xp = between(body, "profile_xp_block_xp\">", "<").map(clean).unwrap_or_default();
    let next = between(body, "profile_xp_block_remaining\">", "<").map(clean).unwrap_or_default();
    let progress = between(body, "profile_xp_block_remaining_bar_progress\" style=\"width: ", "%").and_then(|p| p.trim().parse::<f32>().ok()).unwrap_or(0.0) / 100.0;
    let items = body
        .split("class=\"badge_row ")
        .skip(1)
        .filter_map(|row| {
            let url = between(row, "badge_row_overlay\" href=\"", "\"").unwrap_or_default().to_string();
            let icon = between(row, "data-delayed-image=\"", "\"").or_else(|| between(row, "class=\"badge_info_image\">", "</div>").and_then(|b| between(b, "src=\"", "\""))).map(String::from).filter(|u| !u.contains("trans.gif"));
            let info = between(row, "class=\"badge_info_description\">", "<div style=\"clear")?;
            let title = between(info, "class=\"badge_info_title\">", "</div>").map(clean).unwrap_or_default();
            let unlocked = between(info, "class=\"badge_info_unlocked\">", "</div>").map(clean).unwrap_or_default();
            let unlocked_at = unlock_time(&unlocked, this_year()).unwrap_or(0);
            let playtime = between(row, "class=\"badge_title_stats_playtime\">", "</div>").map(clean).unwrap_or_default();
            let drops = drops_text(row);
            // The line between title and unlock date: level / XP.
            let after_title = info.split("badge_info_title").nth(1).unwrap_or_default();
            let detail = between(after_title, "<div>", "</div>").map(clean).unwrap_or_default();
            (!title.is_empty()).then_some(model::Badge { title, icon, detail, unlocked, unlocked_at, url, playtime, drops })
        })
        .collect();
    model::Badges { level, xp, next, progress, items }
}

/// A badge's own page (`…/badges/<n>` or `…/gamecards/<app>/`).
pub async fn badge(web: Option<&Web>, url: &str) -> Result<model::BadgeDetail, String> {
    if !url.starts_with(COMMUNITY) {
        return Err("not a Steam Community badge".into());
    }
    let body = http(web).get(url).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let mut d = parse_badge(&body);
    d.app_id = url.split("/gamecards/").nth(1).and_then(|r| r.split('/').next()).and_then(|a| a.parse().ok());
    d.current.url = url.to_string();
    Ok(d)
}

pub fn parse_badge(body: &str) -> model::BadgeDetail {
    let title = between(body, "class=\"badge_title\">", "</div>").map(clean).unwrap_or_default();
    let info = between(body, "class=\"badge_info_description\">", "<div style=\"clear").unwrap_or_default();
    let item_title = between(info, "class=\"badge_info_title\">", "</div>").map(clean).unwrap_or_default();
    let unlocked = between(info, "class=\"badge_info_unlocked\">", "</div>").map(clean).unwrap_or_default();
    let after_title = info.split("badge_info_title").nth(1).unwrap_or_default();
    let current = model::Badge {
        title: item_title,
        icon: between(body, "class=\"badge_info_image\">", "</div>").and_then(|b| between(b, "src=\"", "\"")).map(String::from).filter(|u| !u.contains("trans.gif")),
        detail: between(after_title, "<div>", "</div>").map(clean).unwrap_or_default(),
        unlocked_at: unlock_time(&unlocked, this_year()).unwrap_or(0),
        unlocked,
        playtime: between(body, "class=\"badge_title_stats_playtime\">", "</div>").map(clean).unwrap_or_default(),
        drops: drops_text(body),
        ..Default::default()
    };
    let cards = body
        .split("class=\"badge_card_set_card ")
        .skip(1)
        .map(|c| {
            let name_block = between(c, "badge_card_set_title ellipsis\">", "<div style=\"clear").unwrap_or_default();
            let owned = between(name_block, "badge_card_set_text_qty\">", "</div>").map(|q| q.trim_matches(|ch: char| !ch.is_ascii_digit()).parse().unwrap_or(1)).unwrap_or(0);
            let name = match name_block.split_once("</div>") {
                Some((_, rest)) if owned > 0 => clean(rest),
                _ => clean(name_block),
            };
            let position = c.split("badge_card_set_title").nth(1).and_then(|r| r.split("class=\"badge_card_set_text ellipsis\">").nth(1)).and_then(|r| r.split("</div>").next()).map(clean).unwrap_or_default();
            model::BadgeCard { name, image: between(c, "class=\"gamecard\" src=\"", "\"").unwrap_or_default().to_string(), owned, position }
        })
        .collect();
    model::BadgeDetail { title, current, cards, app_id: None }
}

/// Craft the next level of a game's badge from a full card set. Returns
/// the new badge's name, as Steam reports it.
pub async fn craft_badge(web: &Web, app: model::AppId, foil: bool) -> Result<String, String> {
    let v: Value = web
        .http
        .post(format!("{COMMUNITY}/profiles/{}/ajaxcraftbadge/", web.me))
        .header("Referer", format!("{COMMUNITY}/profiles/{}/gamecards/{app}/", web.me))
        .form(&[("appid", app.to_string()), ("series", "1".into()), ("border_color", if foil { "1" } else { "0" }.into()), ("levels", "1".into()), ("sessionid", web.session_id.clone())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    match v.get("success").and_then(|s| s.as_i64()) {
        Some(1) => Ok(v.get("Badge").and_then(|b| b.get("title")).and_then(|t| t.as_str()).unwrap_or_default().to_string()),
        _ => Err("Steam didn't craft the badge. You need one of every card in the set.".into()),
    }
}

/// "3 card drops remaining" from a badge's stats (without Steam's help link
/// and "Card drops earned" after it).
fn drops_text(block: &str) -> String {
    let Some(d) = between(block, "class=\"badge_title_stats_drops\">", "</div>") else { return String::new() };
    match between(d, "class=\"progress_info_bold\">", "</span>") {
        Some(b) => clean(b),
        None => clean(d),
    }
}

fn this_year() -> i64 {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86_400).unwrap_or(0);
    // The year of a day count (1970-based), by stepping a year at a time.
    let mut y = 1970;
    while model::days_from_civil(y + 1, 1, 1) <= days {
        y += 1;
    }
    y
}

/// "Unlocked Jan 13, 2025 @ 7:17am" or "Unlocked Oct 14 @ 10:06am" (this
/// year) as a wall-clock time: seconds since 1970 read as UTC.
pub fn unlock_time(text: &str, year_now: i64) -> Option<u64> {
    let rest = text.trim().strip_prefix("Unlocked").unwrap_or(text).trim();
    let (date, time) = rest.split_once('@')?;
    // "Jan 13, 2025" or "13 Jan, 2025" (Steam orders it by your region).
    let mut month = None;
    let (mut day, mut year) = (None, None);
    for p in date.split(|c: char| c == ' ' || c == ',').filter(|p| !p.is_empty()) {
        let lower = p.to_lowercase();
        if let Some(m) = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| lower.starts_with(m)) {
            month = Some(m as i64 + 1);
        } else if let Ok(n) = p.parse::<i64>() {
            if n > 31 { year = Some(n) } else { day = Some(n) }
        }
    }
    let (month, day, year) = (month?, day?, year.unwrap_or(year_now));
    let time = time.trim().to_lowercase();
    let pm = time.ends_with("pm");
    let (h, m) = time.trim_end_matches(|c: char| c.is_ascii_alphabetic()).trim().split_once(':')?;
    let (mut h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    if h == 12 {
        h = 0;
    }
    if pm {
        h += 12;
    }
    if !(1..=31).contains(&day) || h > 23 || m > 59 {
        return None;
    }
    Some((model::days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60) as u64)
}

/// Someone's friends (their profile's friends page, which needs to be
/// visible to you).
pub async fn profile_friends(web: Option<&Web>, steam_id: SteamId) -> Result<Vec<SteamId>, String> {
    let body = http(web).get(format!("{COMMUNITY}/profiles/{steam_id}/friends/")).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let ids = parse_friend_ids(&body, steam_id);
    if ids.is_empty() && body.contains("profile_private_info") {
        return Err("private".into());
    }
    Ok(ids)
}

/// The SteamIDs on a friends page (each friend block carries one), in order.
pub fn parse_friend_ids(body: &str, owner: SteamId) -> Vec<SteamId> {
    let mut out: Vec<SteamId> = Vec::new();
    for part in body.split("data-steamid=\"").skip(1) {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = digits.parse::<SteamId>() {
            if id != owner && id > 76_561_197_960_265_728 && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// The latest reviews on someone's profile (Steam's first page of them).
pub async fn profile_reviews(web: Option<&Web>, steam_id: SteamId) -> Result<Vec<model::ProfileReview>, String> {
    let body = http(web).get(format!("{COMMUNITY}/profiles/{steam_id}/recommended/")).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let list = parse_reviews(&body);
    if list.is_empty() && body.contains("profile_private_info") {
        return Err("private".into());
    }
    Ok(list)
}

pub fn parse_reviews(body: &str) -> Vec<model::ProfileReview> {
    // `review_box">` exactly: there's also a `review_box_content` inside each.
    body.split("class=\"review_box\">")
        .skip(1)
        .filter_map(|row| {
            let app = between(row, "/recommended/", "/").or_else(|| between(row, "steamcommunity.com/app/", "\""))?;
            let app_id = app.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
            // `class="content ">`: Steam leaves a space in the class.
            let text = row.split("class=\"content").nth(1).and_then(|c| c.split_once('>')).and_then(|(_, c)| c.split("</div>").next()).map(clean).unwrap_or_default();
            // "12 people found this review helpful<br>1 person found it funny", or one line.
            let helpful = between(row, "class=\"header\">", "</div>").map(|h| clean(h.split("<br").next().unwrap_or(h))).unwrap_or_default();
            Some(model::ProfileReview {
                app_id,
                voted_up: !row.contains("icon_thumbsDown"),
                hours: between(row, "class=\"hours\">", "<").map(clean).unwrap_or_default(),
                posted: between(row, "class=\"posted\">", "<").or_else(|| between(row, "class=\"date_posted\">", "<")).map(clean).unwrap_or_default(),
                text: text.chars().take(1200).collect(),
                helpful,
            })
        })
        .collect()
}

/// The groups someone is in (from their profile's XML).
pub async fn profile_groups(web: Option<&Web>, steam_id: SteamId) -> Result<Vec<model::ProfileGroup>, String> {
    let body = http(web).get(format!("{COMMUNITY}/profiles/{steam_id}/?xml=1")).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    if !body.contains("<groups>") && body.contains("<privacyState>private") {
        return Err("private".into());
    }
    Ok(parse_groups(&body))
}

pub fn parse_groups(xml: &str) -> Vec<model::ProfileGroup> {
    let field = |g: &str, tag: &str| -> String {
        let raw = between(g, &format!("<{tag}>"), &format!("</{tag}>")).unwrap_or_default();
        let raw = raw.trim().trim_start_matches("<![CDATA[").trim_end_matches("]]>");
        clean(raw)
    };
    xml.split("<group ")
        .skip(1)
        .filter_map(|g| {
            let id = field(g, "groupID64").parse().ok()?;
            Some(model::ProfileGroup {
                id,
                name: field(g, "groupName"),
                url: field(g, "groupURL"),
                headline: field(g, "headline"),
                avatar: field(g, "avatarMedium"),
                members: field(g, "memberCount").replace(',', "").parse().unwrap_or(0),
                online: field(g, "membersOnline").replace(',', "").parse().unwrap_or(0),
                primary: g.starts_with("isPrimary=\"1\""),
            })
        })
        .collect()
}

/// A game's Workshop tag groups, as its Workshop page declares them.
pub async fn workshop_tags(app: model::AppId, language: &str) -> Result<Vec<model::WorkshopTagCategory>, String> {
    let body = reqwest::Client::new()
        .get(format!("{COMMUNITY}/workshop/browse/"))
        .query(&[("appid", app.to_string()), ("l", language.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_workshop_tags(&body)
}

pub fn parse_workshop_tags(body: &str) -> Result<Vec<model::WorkshopTagCategory>, String> {
    // The page embeds its data as an escaped JSON string.
    let marker = r#"\"declaredTags\":"#;
    let at = body.find(marker).ok_or("This game's Workshop has no tags")? + marker.len();
    let mut text = String::new();
    let mut chars = body[at..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    text.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('?'));
                }
                Some(other) => text.push(other),
                None => break,
            },
            '"' => break,
            _ => text.push(c),
        }
    }
    let v: Value = serde_json::Deserializer::from_str(&text).into_iter::<Value>().next().ok_or("bad tag data")?.map_err(|e| e.to_string())?;
    // Regular items first; games that only sell items use the store's groups.
    let groups = ["readytouse_tags", "mtx_tags", "collection_tags"]
        .iter()
        .filter_map(|k| v.get(*k)?.as_array())
        .find(|a| !a.is_empty())
        .cloned()
        .unwrap_or_default();
    Ok(groups
        .iter()
        .filter_map(|g| {
            let tags: Vec<(String, String)> = g
                .get("tags")?
                .as_array()?
                .iter()
                .filter(|t| !t.get("admin_only").and_then(|a| a.as_bool()).unwrap_or(false))
                .filter_map(|t| {
                    let name = t.get("name")?.as_str()?.to_string();
                    let shown = t.get("display_name").and_then(|d| d.as_str()).filter(|d| !d.is_empty()).unwrap_or(&name).to_string();
                    Some((name, shown))
                })
                .collect();
            let name = g.get("name").and_then(|n| n.as_str()).unwrap_or_default().trim_start_matches('#').to_string();
            (!tags.is_empty()).then_some(model::WorkshopTagCategory { name, single: g.get("htmlelement").and_then(|h| h.as_str()) == Some("select"), tags })
        })
        .collect())
}

/// Library collections from Steam's synced settings ("user-collections.*").
pub fn collections<'a>(entries: impl Iterator<Item = (&'a str, bool, &'a str)>) -> Vec<model::SteamCollection> {
    let mut out: Vec<model::SteamCollection> = entries
        .filter(|(key, deleted, _)| key.starts_with("user-collections.") && !deleted)
        .filter_map(|(_, _, value)| {
            let v: Value = serde_json::from_str(value).ok()?;
            let id = v.get("id")?.as_str()?.to_string();
            let added: Vec<u32> = v.get("added").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
            let removed: Vec<u32> = v.get("removed").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
            // The filters exactly as Steam wrote them (parsing would reorder them).
            let filter = crate::collections::raw_field(value, "filterSpec").filter(|f| f != "null");
            Some(model::SteamCollection {
                name: v.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                games: added.iter().copied().filter(|a| !removed.contains(a)).collect(),
                dynamic: filter.is_some(),
                filter,
                added,
                removed,
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_steam_collections() {
        let entries = [
            ("user-collections.favorite", false, r#"{"id":"favorite","name":"Favorites","added":[570,620,730],"removed":[730]}"#),
            ("user-collections.uc-1", true, r#"{"id":"uc-1","name":"Gone","added":[1]}"#),
            ("showcases.1", false, "{}"),
        ];
        let c = super::collections(entries.into_iter());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "favorite");
        assert_eq!(c[0].games, vec![570, 620]);
    }

    #[test]
    fn reads_workshop_tag_groups() {
        let page = r#"x = "{\"declaredTags\":{\"guide_tags\":[],\"readytouse_tags\":[{\"name\":\"Content Type\",\"htmlelement\":\"select\",\"tags\":[{\"id\":\"1\",\"name\":\"Addon\",\"display_name\":\"Addon\",\"admin_only\":false}]},{\"name\":\"Addon Type\",\"htmlelement\":\"checkbox\",\"tags\":[{\"id\":\"2\",\"name\":\"Weapon\",\"display_name\":\"Weapon\",\"admin_only\":false},{\"id\":\"3\",\"name\":\"Secret\",\"display_name\":\"Secret\",\"admin_only\":true}]}]}}";"#;
        let cats = super::parse_workshop_tags(page).unwrap();
        assert_eq!(cats.len(), 2);
        assert!(cats[0].single);
        assert_eq!(cats[1].tags, vec![("Weapon".to_string(), "Weapon".to_string())]);
    }

    #[test]
    fn reads_comments_and_badges() {
        let c = r#"<div class="commentthread_comment responsive_body_text   " id="comment_123"><div class="commentthread_comment_content"><div class="commentthread_comment_author"><div class="commentthread_comment_avatar playerAvatar"><a href="https://steamcommunity.com/profiles/76561197960265730" data-miniprofile="2"><img src="https://avatars.example/a.jpg"></a></div><div class="author_name_group"><a class="hoverunderline commentthread_author_link" href="x" data-miniprofile="2"><bdi>Tester</bdi></a><div class="commentthread_comment_timestamp" data-timestamp="1788630100">Sep 5</div></div></div><div class="commentthread_comment_text" id="comment_content_123"> hello &amp; welcome </div></div></div>"#;
        let list = super::parse_comments(c);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].author, 76561197960265730);
        assert_eq!(list[0].name, "Tester");
        assert_eq!(list[0].time, 1788630100);
        assert_eq!(list[0].avatar.as_deref(), Some("https://avatars.example/a.jpg"));
        assert_eq!(list[0].html, "hello &amp; welcome");

        let b = r#"<span class="friendPlayerLevelNum">18</span><span class="profile_xp_block_xp">XP 2,715</span><div class="profile_xp_block_remaining">85 XP to reach Level 19</div><div class="profile_xp_block_remaining_bar_progress" style="width: 58%"></div>
        <div class="badge_row is_link"><a class="badge_row_overlay" href="https://x/badges/13"></a><div class="badge_info_image"><img src="trans.gif" data-delayed-image="https://x/13.png" class="badge_icon"></div><div class="badge_info_description"><div class="badge_info_title">Power Player</div><div> 340 XP </div><div class="badge_info_unlocked"> Unlocked Oct 14 @ 10:06am </div></div><div style="clear: both;"></div></div>"#;
        let badges = super::parse_badges(b);
        assert_eq!(badges.level, 18);
        assert_eq!(badges.items[0].unlocked_at, super::unlock_time("Unlocked Oct 14 @ 10:06am", super::this_year()).unwrap());
        // 13 Jan 2025 07:17 and 14 Oct 2026 22:06.
        assert_eq!(super::unlock_time("Unlocked Jan 13, 2025 @ 7:17am", 2026), Some(1_736_752_620));
        assert_eq!(super::unlock_time("Unlocked Oct 14 @ 10:06pm", 2026), Some(1_792_015_560));
        assert_eq!(super::unlock_time("Unlocked Jan 1, 2020 @ 12:05am", 2026), Some(1_577_836_800 + 300));
        assert_eq!(super::unlock_time("Unlocked 13 Jan, 2025 @ 7:17am", 2026), Some(1_736_752_620));
        assert_eq!(super::unlock_time("Unlocked 14 Oct @ 10:06pm", 2026), Some(1_792_015_560));
        assert_eq!(super::unlock_time("whenever", 2026), None);
        let page = r#"<div class="badge_title"> Dota 2 Badge </div><div class="badge_info"><div class="badge_info_image"><img src="https://x/570/b.png" class="badge_icon"></div><div class="badge_info_description"><div class="badge_info_title">Disabler</div><div> Level 2, 200 XP </div><div class="badge_info_unlocked"> Unlocked 13 Jan, 2025 @ 7:17am </div></div><div style="clear: inline-start;"></div></div>
            <div class="badge_card_set_card unowned"><div class="game_card_ctn "><img class="gamecard" src="https://x/c1"></div><div class="badge_card_set_text badge_card_set_title ellipsis"> Bounty Hunter <div style="clear: inline-end"></div></div><div class="badge_card_set_text ellipsis"> 1 of 8, Series 1 </div></div>
            <div class="badge_card_set_card owned"><div class="game_card_ctn "><img class="gamecard" src="https://x/c3"></div><div class="badge_card_set_text badge_card_set_title ellipsis"> <div class="badge_card_set_text_qty">(2)</div> Razor <div style="clear: inline-end"></div></div><div class="badge_card_set_text ellipsis"> 3 of 8, Series 1 </div></div>"#;
        let d = super::parse_badge(page);
        assert_eq!((d.title.as_str(), d.current.title.as_str(), d.current.detail.as_str()), ("Dota 2 Badge", "Disabler", "Level 2, 200 XP"));
        assert_eq!(d.cards.len(), 2);
        assert_eq!((d.cards[0].name.as_str(), d.cards[0].owned, d.cards[0].position.as_str()), ("Bounty Hunter", 0, "1 of 8, Series 1"));
        assert_eq!((d.cards[1].name.as_str(), d.cards[1].owned, d.cards[1].image.as_str()), ("Razor", 2, "https://x/c3"));
        assert_eq!(badges.next, "85 XP to reach Level 19");
        assert!((badges.progress - 0.58).abs() < 0.01);
        assert_eq!(badges.items.len(), 1);
        assert_eq!(badges.items[0].title, "Power Player");
        assert_eq!(badges.items[0].detail, "340 XP");
        assert_eq!(badges.items[0].icon.as_deref(), Some("https://x/13.png"));
    }
}

#[cfg(test)]
mod profile_page_tests {
    #[test]
    fn reads_friends_reviews_and_groups() {
        let friends = r#"<div class="selectable friend_block_v2 persona online" data-steamid="76561198000000001" data-search="a"></div>
            <div class="selectable friend_block_v2 persona offline" data-steamid="76561198000000002"></div>
            <a data-steamid="76561198000000001"></a><div data-steamid="76561198099999999"></div>"#;
        assert_eq!(super::parse_friend_ids(friends, 76561198099999999), vec![76561198000000001, 76561198000000002]);

        let reviews = r#"<div class="review_box">
            <div class="leftcol"><a href="https://steamcommunity.com/app/570"><img src="x"></a>
            <div class="hours">3,773.1 hrs on record</div></div>
            <div class="rightcol"><div class="header">12 people found this review helpful<br>1 person found this review funny</div>
            <div class="vote_header"><div class="thumb"><a href="https://steamcommunity.com/id/x/recommended/570/"><img src="https://x/icon_thumbsUp_v6.png"></a></div>
            <div class="title"><a href="https://steamcommunity.com/id/x/recommended/570/">Recommended</a></div></div>
            <div class="posted">Posted 12 May, 2021.</div>
            <div class="content ">Great &amp; <b>fun</b></div></div></div>
            <div class="review_box"><div class="header">
            No one has rated this review as helpful yet
            </div><div class="review_box_content"><a href="https://steamcommunity.com/id/x/recommended/730/"><img src="icon_thumbsDown.png"></a><div class="content ">No</div></div></div>"#;
        let r = super::parse_reviews(reviews);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].app_id, r[0].voted_up, r[0].hours.as_str(), r[0].text.as_str()), (570, true, "3,773.1 hrs on record", "Great & fun"));
        assert_eq!(r[0].helpful, "12 people found this review helpful");
        assert_eq!(r[0].posted, "Posted 12 May, 2021.");
        assert_eq!((r[1].app_id, r[1].voted_up, r[1].text.as_str()), (730, false, "No"));
        assert_eq!(r[1].helpful, "No one has rated this review as helpful yet");

        let xml = r#"<groups><group isPrimary="1">
            <groupID64>103582791430059237</groupID64>
            <groupName><![CDATA[Ludum Dare]]></groupName>
            <groupURL><![CDATA[ludumdare]]></groupURL>
            <headline><![CDATA[Game jam]]></headline>
            <avatarMedium><![CDATA[https://avatars.fastly.steamstatic.com/a_medium.jpg]]></avatarMedium>
            <memberCount>12,345</memberCount><membersOnline>67</membersOnline>
            </group><group isPrimary="0"><groupID64>103582791429521408</groupID64><groupName><![CDATA[Valve]]></groupName></group></groups>"#;
        let g = super::parse_groups(xml);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].name.as_str(), g[0].url.as_str(), g[0].members, g[0].online, g[0].primary), ("Ludum Dare", "ludumdare", 12345, 67, true));
        assert!(!g[1].primary);
    }
}

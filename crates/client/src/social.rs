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
            // The line between title and unlock date: level / XP.
            let after_title = info.split("badge_info_title").nth(1).unwrap_or_default();
            let detail = between(after_title, "<div>", "</div>").map(clean).unwrap_or_default();
            (!title.is_empty()).then_some(model::Badge { title, icon, detail, unlocked, url })
        })
        .collect();
    model::Badges { level, xp, next, progress, items }
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
            Some(model::SteamCollection {
                name: v.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                games: added.into_iter().filter(|a| !removed.contains(a)).collect(),
                dynamic: v.get("filterSpec").is_some(),
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
        assert_eq!(badges.next, "85 XP to reach Level 19");
        assert!((badges.progress - 0.58).abs() < 0.01);
        assert_eq!(badges.items.len(), 1);
        assert_eq!(badges.items[0].title, "Power Player");
        assert_eq!(badges.items[0].detail, "340 XP");
        assert_eq!(badges.items[0].icon.as_deref(), Some("https://x/13.png"));
    }
}

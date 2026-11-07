//! Game update posts ("events"), as shown on a game's library page, and
//! workshop item details. Bodies are Steam BBCode, parsed to layout blocks.

use crate::html::Block;
use crate::{Result, StoreClient};
use model::AppId;
use serde::Deserialize;

const CLAN_IMAGES: &str = "https://clan.akamai.steamstatic.com/images";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    SmallUpdate,
    Update,
    MajorUpdate,
    News,
    InGameEvent,
    Other,
}

impl EventKind {
    fn from_id(id: u32) -> Self {
        match id {
            12 => Self::SmallUpdate,
            13 => Self::Update,
            14 => Self::MajorUpdate,
            28 => Self::News,
            9 | 10 | 11 | 15 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 27 | 30 | 31 | 32 | 33 | 35 => Self::InGameEvent,
            _ => Self::Other,
        }
    }

    /// English label (translate with i18n).
    pub fn label(self) -> &'static str {
        match self {
            Self::SmallUpdate => "Small update",
            Self::Update => "Regular update",
            Self::MajorUpdate => "Major update",
            Self::News => "News",
            Self::InGameEvent => "In-game event",
            Self::Other => "Announcement",
        }
    }

    pub fn is_update(self) -> bool {
        matches!(self, Self::SmallUpdate | Self::Update | Self::MajorUpdate)
    }
}

#[derive(Debug, Clone)]
pub struct GameEvent {
    pub gid: String,
    pub title: String,
    pub kind: EventKind,
    /// Unix seconds.
    pub time: u64,
    /// Wide card image.
    pub capsule: Option<String>,
    /// Header image shown above the post.
    pub header: Option<String>,
    pub body: Vec<Block>,
    pub summary: String,
    pub votes_up: u32,
    pub comments: u32,
}

#[derive(Debug, Clone, Default)]
pub struct WorkshopItem {
    pub id: u64,
    pub title: String,
    pub preview: Option<String>,
    pub size: u64,
    pub time_updated: u64,
    pub subscriptions: u64,
}

impl StoreClient {
    /// Recent update posts for a game, newest first.
    pub async fn events(&self, app: AppId, count: u32) -> Result<Vec<GameEvent>> {
        let lang = match self.language.as_str() {
            "russian" => "8_0",
            _ => "0",
        };
        let resp: serde_json::Value = self
            .http
            .get("https://store.steampowered.com/events/ajaxgetadjacentpartnerevents/")
            .query(&[("appid", app.to_string()), ("count_before", "0".into()), ("count_after", count.to_string()), ("lang_list", lang.into())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // One post at a time, so a single odd one can't empty the feed.
        let events = resp.get("events").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        Ok(events
            .into_iter()
            .filter_map(|e| serde_json::from_value::<RawEvent>(e).ok())
            .filter(|e| e.hidden == 0)
            .map(GameEvent::from)
            .collect())
    }

    /// Titles, previews and sizes for workshop items.
    pub async fn workshop_details(&self, ids: &[u64]) -> Result<Vec<WorkshopItem>> {
        let mut form: Vec<(String, String)> = vec![("itemcount".into(), ids.len().to_string())];
        for (i, id) in ids.iter().enumerate() {
            form.push((format!("publishedfileids[{i}]"), id.to_string()));
        }
        let resp: RawFileEnvelope = self
            .http
            .post("https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/")
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .response
            .publishedfiledetails
            .into_iter()
            .filter(|f| f.result == 1)
            .map(|f| WorkshopItem {
                id: f.publishedfileid.parse().unwrap_or(0),
                title: f.title,
                preview: f.preview_url.filter(|p| !p.is_empty()),
                size: f.file_size.parse().unwrap_or(0),
                time_updated: f.time_updated,
                subscriptions: f.subscriptions,
            })
            .collect())
    }
}

impl From<RawEvent> for GameEvent {
    fn from(e: RawEvent) -> Self {
        let clan = e.announcement_body.clanid;
        let json: RawEventJson = serde_json::from_str(&e.jsondata).unwrap_or_default();
        let image = |list: &[Option<String>]| {
            list.iter().flatten().find(|s| !s.is_empty()).map(|s| format!("{CLAN_IMAGES}/{clan}/{s}"))
        };
        let body = parse_bbcode(&e.announcement_body.body);
        let summary = body
            .iter()
            .find_map(|b| match b {
                Block::Paragraph(runs) => Some(runs.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(" ")),
                _ => None,
            })
            .unwrap_or_default();
        GameEvent {
            gid: e.gid,
            title: if e.announcement_body.headline.is_empty() { e.event_name } else { e.announcement_body.headline },
            kind: EventKind::from_id(e.event_type),
            time: e.rtime32_start_time,
            capsule: image(&json.localized_capsule_image),
            header: image(&json.localized_title_image),
            body,
            summary,
            votes_up: e.votes_up as u32,
            comments: e.comment_count as u32,
        }
    }
}

/// Steam BBCode → layout blocks. Handles the tags Steam posts use.
pub fn parse_bbcode(src: &str) -> Vec<Block> {
    let src = src
        .replace("{STEAM_CLAN_LOC_IMAGE}", CLAN_IMAGES)
        .replace("{STEAM_CLAN_IMAGE}", CLAN_IMAGES);
    let mut blocks = Vec::new();
    let mut runs: Vec<(String, bool)> = Vec::new();
    let mut bold = 0i32;
    let mut heading = false;
    let mut in_item = false;
    let mut buf = String::new();

    fn push(runs: &mut Vec<(String, bool)>, buf: &mut String, bold: bool) {
        let t = crate::html::text(buf);
        buf.clear();
        if t.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some((prev, b)) if *b == bold => {
                prev.push(' ');
                prev.push_str(&t);
            }
            _ => runs.push((t, bold)),
        }
    }
    fn flush(blocks: &mut Vec<Block>, runs: &mut Vec<(String, bool)>, item: bool) {
        if !runs.is_empty() {
            let r = std::mem::take(runs);
            blocks.push(if item { Block::Bullet(r) } else { Block::Paragraph(r) });
        }
    }

    let mut rest = src.as_str();
    while let Some(start) = rest.find('[') {
        buf.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(']') else {
            buf.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let lower = tag.to_ascii_lowercase();
        let closing = lower.starts_with('/');
        let name: String = lower.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '*').collect();
        rest = after;
        match name.as_str() {
            "b" | "u" => {
                push(&mut runs, &mut buf, bold > 0);
                bold = (bold + if closing { -1 } else { 1 }).max(0);
            }
            "h1" | "h2" | "h3" => {
                if closing {
                    let t = crate::html::text(&buf);
                    buf.clear();
                    if !t.is_empty() {
                        blocks.push(Block::Heading(t));
                    }
                    heading = false;
                } else {
                    push(&mut runs, &mut buf, bold > 0);
                    flush(&mut blocks, &mut runs, in_item);
                    heading = true;
                }
            }
            "p" | "list" | "olist" | "hr" | "quote" | "table" | "tr" if !heading => {
                push(&mut runs, &mut buf, bold > 0);
                flush(&mut blocks, &mut runs, in_item);
                if name == "list" || name == "olist" {
                    in_item = false;
                }
            }
            "*" => {
                push(&mut runs, &mut buf, bold > 0);
                flush(&mut blocks, &mut runs, in_item);
                in_item = true;
            }
            "img" if !closing => {
                push(&mut runs, &mut buf, bold > 0);
                flush(&mut blocks, &mut runs, in_item);
                // [img src="..."][/img] or [img]url[/img]
                let url = attr(tag, "src").or_else(|| {
                    let close = rest.to_ascii_lowercase().find("[/img]")?;
                    let u = rest[..close].trim().to_string();
                    rest = &rest[close + 6..];
                    Some(u)
                });
                if let Some(u) = url.filter(|u| u.starts_with("http")) {
                    blocks.push(Block::Image(u));
                }
            }
            "previewyoutube" if !closing => {
                push(&mut runs, &mut buf, bold > 0);
                flush(&mut blocks, &mut runs, in_item);
                if let Some(v) = tag.split('=').nth(1) {
                    let id = v.trim_matches('"').split(';').next().unwrap_or_default();
                    if !id.is_empty() {
                        blocks.push(Block::Video(id.to_string()));
                    }
                }
            }
            "url" if !closing => {
                // Keep the link text; the URL itself isn't shown inline.
            }
            _ => {}
        }
    }
    buf.push_str(rest);
    push(&mut runs, &mut buf, bold > 0);
    flush(&mut blocks, &mut runs, in_item);
    blocks
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let r = &tag[i..];
    Some(r[..r.find('"')?].to_string())
}

fn int_or_str<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0))
}

#[derive(Deserialize)]
struct RawEvent {
    #[serde(default)]
    gid: String,
    #[serde(default)]
    event_name: String,
    #[serde(default)]
    event_type: u32,
    #[serde(default)]
    rtime32_start_time: u64,
    #[serde(default)]
    jsondata: String,
    #[serde(default, deserialize_with = "int_or_str")]
    hidden: u64,
    #[serde(default, deserialize_with = "int_or_str")]
    votes_up: u64,
    #[serde(default, deserialize_with = "int_or_str")]
    comment_count: u64,
    #[serde(default)]
    announcement_body: RawBody,
}

#[derive(Deserialize, Default)]
struct RawBody {
    /// Steam sends this as a number or a string.
    #[serde(default, deserialize_with = "int_or_str")]
    clanid: u64,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize, Default)]
struct RawEventJson {
    #[serde(default)]
    localized_capsule_image: Vec<Option<String>>,
    #[serde(default)]
    localized_title_image: Vec<Option<String>>,
}

#[derive(Deserialize)]
struct RawFileEnvelope {
    response: RawFiles,
}

#[derive(Deserialize)]
struct RawFiles {
    #[serde(default)]
    publishedfiledetails: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    publishedfileid: String,
    #[serde(default)]
    result: i32,
    #[serde(default)]
    title: String,
    preview_url: Option<String>,
    #[serde(default, deserialize_with = "num_or_str")]
    file_size: String,
    #[serde(default)]
    time_updated: u64,
    #[serde(default)]
    subscriptions: u64,
}

fn num_or_str<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steam_bbcode() {
        let b = parse_bbcode(r#"[h2]BLUEPRINTS[/h2][p]Workbenches are [b]free[/b] now.[/p][list][*]One[*]Two[/list][img src="{STEAM_CLAN_IMAGE}/1/a.png"][/img][previewyoutube="ZF09w_v7Shw;full"][/previewyoutube]"#);
        assert_eq!(b[0], Block::Heading("BLUEPRINTS".into()));
        assert_eq!(b[1], Block::Paragraph(vec![("Workbenches are".into(), false), ("free".into(), true), ("now.".into(), false)]));
        assert_eq!(b[2], Block::Bullet(vec![("One".into(), false)]));
        assert_eq!(b[3], Block::Bullet(vec![("Two".into(), false)]));
        assert_eq!(b[4], Block::Image("https://clan.akamai.steamstatic.com/images/1/a.png".into()));
        assert_eq!(b[5], Block::Video("ZF09w_v7Shw".into()));
    }
}

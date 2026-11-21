//! Doing things in the community without Steam's website: starting a
//! discussion topic, joining and leaving groups, answering group invites.
//!
//! The requests are the ones Steam's own pages send (`forums.js`'s
//! `createtopic`, the group page's join form, `friends.js`'s `leave_group`)
//! and the message its client sends for an invite. They're built and read
//! by plain functions here, so tests can check them without touching an
//! account. Whether you're in a group comes from Steam's friends list.

use crate::hub::{decode, group_path, strip_tags};
use crate::market::Web;
use model::{AppId, SteamId};

const COMMUNITY: &str = "https://steamcommunity.com";

/// Where you stand with a group (Steam's `EClanRelationship`, sent with the
/// friends list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClanRelationship {
    #[default]
    None,
    Blocked,
    Invited,
    Member,
    Kicked,
    KickAcknowledged,
    PendingApproval,
    RequestDenied,
    Other(u32),
}

impl ClanRelationship {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Blocked,
            2 => Self::Invited,
            3 => Self::Member,
            4 => Self::Kicked,
            5 => Self::KickAcknowledged,
            6 => Self::PendingApproval,
            7 => Self::RequestDenied,
            other => Self::Other(other),
        }
    }
}

/// A Steam group's id (account type 7), as opposed to a person's.
pub fn is_clan(id: SteamId) -> bool {
    (id >> 52) & 0xF == 7
}

/// How a group takes new members, from its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum JoinKind {
    /// The page didn't say (you may already be in it).
    #[default]
    Unknown,
    Open,
    /// "Request To Join": an officer has to approve.
    Request,
    /// "Membership by invitation only".
    InviteOnly,
}

/// What the button on a group's page offers, from where you stand with
/// the group and how it takes members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupButton {
    Join,
    RequestToJoin,
    /// No way in without an invite.
    InviteOnly,
    /// You're invited: accept or ignore.
    Invited,
    Leave,
    /// Your request is waiting for an officer.
    RequestPending,
    RequestDenied,
    /// An officer removed you.
    Removed,
    Blocked,
}

pub fn group_button(relationship: ClanRelationship, join: JoinKind) -> GroupButton {
    match relationship {
        ClanRelationship::Member => GroupButton::Leave,
        ClanRelationship::Invited => GroupButton::Invited,
        ClanRelationship::PendingApproval => GroupButton::RequestPending,
        ClanRelationship::RequestDenied => GroupButton::RequestDenied,
        ClanRelationship::Kicked | ClanRelationship::KickAcknowledged => GroupButton::Removed,
        ClanRelationship::Blocked => GroupButton::Blocked,
        ClanRelationship::None | ClanRelationship::Other(_) => match join {
            JoinKind::Request => GroupButton::RequestToJoin,
            JoinKind::InviteOnly => GroupButton::InviteOnly,
            JoinKind::Open | JoinKind::Unknown => GroupButton::Join,
        },
    }
}

/// How a group takes members, from its page in English.
pub fn parse_join_kind(html: &str) -> JoinKind {
    let Some(at) = html.find("grouppage_join_area") else { return JoinKind::Unknown };
    let area = &html[at..];
    let area = &area[..area.char_indices().nth(600).map_or(area.len(), |(i, _)| i)];
    if area.contains("icon_inviteonly") {
        return JoinKind::InviteOnly;
    }
    if !area.contains("join_group_form") {
        return JoinKind::Unknown;
    }
    if area.contains("Request To Join") { JoinKind::Request } else { JoinKind::Open }
}

// ---------- a new discussion topic ----------

/// One of the forums a topic can go in ("Post in:").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Subforum {
    /// Steam's value, e.g. `General_0`.
    pub value: String,
    pub name: String,
    pub selected: bool,
}

/// What a forum page says about starting a topic in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewTopicForm {
    pub owner: String,
    pub forum_type: String,
    pub feature: String,
    /// Game forums send their game's id with the topic.
    pub app_id: Option<AppId>,
    pub forum_url: String,
    pub can_post: bool,
    pub banned: bool,
    /// Steam gave this session the form (it doesn't when you're signed out).
    pub has_form: bool,
    pub subforums: Vec<Subforum>,
}

impl NewTopicForm {
    /// The forum picked at first: the one this page is.
    pub fn default_subforum(&self) -> Option<&Subforum> {
        self.subforums.iter().find(|s| s.selected).or_else(|| self.subforums.first())
    }
}

/// The forum's details from its page (`InitializeForum( "name", {…}, 'url' )`
/// and the `_newtopic_form`).
pub fn parse_new_topic(body: &str) -> Option<NewTopicForm> {
    let at = body.find("InitializeForum(")?;
    let rest = &body[at..];
    let json = &rest[rest.find('{')?..];
    let v: serde_json::Value = serde_json::Deserializer::from_str(json).into_iter().next()?.ok()?;
    let text = |k: &str| match v.get(k) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let flag = |k: &str| v.get("permissions").and_then(|p| p.get(k)).is_some_and(|f| f.as_i64() == Some(1) || f.as_bool() == Some(true));
    let mut form = NewTopicForm {
        owner: text("owner"),
        forum_type: text("type"),
        feature: text("feature"),
        app_id: v.get("appid").and_then(|a| a.as_u64()).filter(|a| *a != 0).map(|a| a as AppId),
        forum_url: text("forum_url"),
        can_post: flag("can_post"),
        banned: flag("is_banned"),
        has_form: false,
        subforums: Vec::new(),
    };
    if form.owner.is_empty() || form.forum_type.is_empty() {
        return None;
    }
    if let Some(i) = body.find("_newtopic_form\"") {
        let html = &body[i..];
        let html = &html[..html.find("</form>").unwrap_or(html.len())];
        form.has_form = html.contains("name=\"topic\"") && html.contains("name=\"text\"");
        if let Some(select) = html.find("name=\"subforum\"").map(|s| &html[s..]) {
            let select = &select[..select.find("</select>").unwrap_or(select.len())];
            for option in select.split("<option").skip(1) {
                let Some((tag, name)) = option.split_once('>') else { continue };
                let Some(value) = tag.split("value=\"").nth(1).and_then(|s| s.split('"').next()) else { continue };
                let name = strip_tags(name.split("</option").next().unwrap_or(name));
                form.subforums.push(Subforum { value: decode(value), name, selected: tag.split_whitespace().any(|w| w == "selected") });
            }
        }
    }
    Some(form)
}

/// The request for a new topic: Steam's page posts its form to
/// `/forum/<owner>/<type>/createtopic/<feature>/`.
pub fn new_topic_request(form: &NewTopicForm, session_id: &str, subject: &str, text: &str, subforum: Option<&str>) -> Result<(String, Vec<(&'static str, String)>), String> {
    if !form.has_form {
        return Err("Sign in to start a discussion".into());
    }
    if form.banned {
        return Err("You're banned from posting in this forum.".into());
    }
    if !form.can_post {
        return Err("You can't post in this forum.".into());
    }
    let (subject, text) = (subject.trim(), text.trim());
    if subject.is_empty() {
        return Err("Give your discussion a title".into());
    }
    if text.is_empty() {
        return Err("Write something in your discussion".into());
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let word = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    if !digits(&form.owner) || !word(&form.forum_type) || !(form.feature.is_empty() || digits(&form.feature)) {
        return Err("This forum can't be posted to here".into());
    }
    let chosen = match subforum {
        Some(value) if form.subforums.iter().any(|s| s.value == value) => Some(value.to_string()),
        Some(_) => return Err("This forum can't be posted to here".into()),
        None => form.default_subforum().map(|s| s.value.clone()),
    };
    let mut url = format!("{COMMUNITY}/forum/{}/{}/createtopic/", form.owner, form.forum_type);
    if !form.feature.is_empty() {
        url.push_str(&form.feature);
        url.push('/');
    }
    let mut fields = vec![("sessionid", session_id.to_string())];
    if let Some(app) = form.app_id {
        fields.push(("appid", app.to_string()));
    }
    fields.push(("topic", subject.to_string()));
    fields.push(("text", text.to_string()));
    if let Some(value) = chosen {
        fields.push(("subforum", value));
    }
    Ok((url, fields))
}

/// Steam's answer to a new topic: the new topic's link, or why not.
pub fn topic_answer(body: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "Steam didn't answer. Your discussion wasn't posted.".to_string())?;
    if let Some(url) = v.get("topic_url").and_then(|u| u.as_str()) {
        return crate::market::validate_steam_url(url, "steamcommunity.com").map(|_| url.to_string()).map_err(|_| "Steam didn't answer. Your discussion wasn't posted.".to_string());
    }
    if let Some(e) = v.get("error").and_then(|e| e.as_str()).map(strip_tags).filter(|e| !e.is_empty()) {
        return Err(e);
    }
    Err(match v.get("success").and_then(|s| s.as_i64()) {
        Some(15) => "You can't post in this forum.".to_string(),
        Some(17) => "You're banned from posting in this forum.".to_string(),
        Some(21) => "Steam signed you out. Sign in again and try once more.".to_string(),
        Some(25 | 84) => "You're posting too often. Wait a little and try again.".to_string(),
        Some(16) => "Steam took too long. Try again.".to_string(),
        Some(code) => format!("Steam didn't take your discussion (error {code})"),
        None => "Steam didn't answer. Your discussion wasn't posted.".to_string(),
    })
}

/// Post a new topic. Answers with its link.
pub async fn new_topic(web: &Web, form: &NewTopicForm, subject: &str, text: &str, subforum: Option<&str>) -> Result<String, String> {
    let (url, fields) = new_topic_request(form, &web.session_id, subject, text, subforum)?;
    let referer = if form.forum_url.starts_with(&format!("{COMMUNITY}/")) { form.forum_url.clone() } else { COMMUNITY.to_string() };
    let r = web.http.post(&url).header("Referer", referer).form(&fields).send().await.map_err(|e| e.to_string())?;
    // Steam says why in the body of its error answers too.
    let body = r.text().await.map_err(|e| e.to_string())?;
    topic_answer(&body)
}

// ---------- joining and leaving ----------

/// The group page's own join form. Used for open groups and for requests
/// to join alike: Steam decides which from the group.
pub fn join_request(key: &str, session_id: &str) -> Result<(String, Vec<(&'static str, String)>), String> {
    let path = group_path(key).ok_or("This isn't a Steam group")?;
    Ok((format!("{COMMUNITY}/{path}"), vec![("action", "join".to_string()), ("sessionID", session_id.to_string())]))
}

/// The join form answers with a page: Steam's error page says why not.
pub fn join_answer(body: &str) -> Result<(), String> {
    let Some(at) = body.find("class=\"error_ctn\"") else { return Ok(()) };
    let area = &body[at..];
    let message = ["<h3>", "class=\"sectionText\">", "<p>"].iter().find_map(|open| {
        let start = area.find(open)? + open.len();
        let end = area[start..].find('<')? + start;
        Some(decode(area[start..end].trim())).filter(|m| !m.is_empty())
    });
    Err(message.unwrap_or_else(|| "Steam didn't let you join this group".to_string()))
}

/// Steam's friends page leaves a group with this (`ApplyFriendAction`).
pub fn leave_request(me: SteamId, session_id: &str, group: SteamId) -> Result<(String, Vec<(&'static str, String)>), String> {
    if me == 0 || !is_clan(group) {
        return Err("This isn't a Steam group".into());
    }
    let fields = vec![
        ("sessionid", session_id.to_string()),
        ("steamid", me.to_string()),
        ("ajax", "1".to_string()),
        ("action", "leave_group".to_string()),
        ("steamids[]", group.to_string()),
    ];
    Ok((format!("{COMMUNITY}/profiles/{me}/friends/action"), fields))
}

pub fn leave_answer(body: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "Steam didn't let you leave this group".to_string())?;
    if v.get("success").and_then(|s| s.as_i64()) == Some(1) { Ok(()) } else { Err("Steam didn't let you leave this group".to_string()) }
}

pub async fn join(web: &Web, key: &str) -> Result<(), String> {
    let (url, fields) = join_request(key, &web.session_id)?;
    let r = web.http.post(&url).header("Referer", url.clone()).form(&fields).send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    join_answer(&body)?;
    if status.is_success() { Ok(()) } else { Err(format!("Steam answered {status}")) }
}

pub async fn leave(web: &Web, me: SteamId, group: SteamId) -> Result<(), String> {
    let (url, fields) = leave_request(me, &web.session_id, group)?;
    let r = web.http.post(&url).header("Referer", format!("{COMMUNITY}/profiles/{me}/groups/")).form(&fields).send().await.map_err(|e| e.to_string())?;
    let body = r.text().await.map_err(|e| e.to_string())?;
    leave_answer(&body)
}

/// The body of `ClientAcknowledgeClanInvite`: the group's id, then 1 to
/// accept or 0 to ignore.
pub fn clan_invite_body(group: SteamId, accept: bool) -> [u8; 9] {
    let mut out = [0u8; 9];
    out[..8].copy_from_slice(&group.to_le_bytes());
    out[8] = u8::from(accept);
    out
}

#[cfg(test)]
mod tests;

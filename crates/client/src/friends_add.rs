//! Ways to add a friend beyond the friend code, like Steam's "Add a friend"
//! page: Quick Invite links (`s.team/p/…`) and searching profiles by name.

use crate::market::Web;
use model::SteamId;
use net::Connection;
use proto::webui::{
    CUserAccountCreateFriendInviteTokenRequest, CUserAccountCreateFriendInviteTokenResponse, CUserAccountGetFriendInviteTokensRequest,
    CUserAccountGetFriendInviteTokensResponse, CUserAccountRedeemFriendInviteTokenRequest, CUserAccountRedeemFriendInviteTokenResponse,
};
use serde_json::Value;

/// A Quick Invite link works once and lasts 30 days, like Steam's.
const INVITE_DAYS: u32 = 30;
const STEAMID_BASE: u64 = 76561197960265728;
/// Steam writes the account id in hex with these letters for the digits,
/// so the short code can't spell words or be confused with a friend code.
const CODE_LETTERS: &[u8; 16] = b"bcdfghjkmnpqrtvw";

/// The short code in a Quick Invite link: "fcvd-whbd" for account 836957442.
pub fn short_code(account_id: u32) -> String {
    let letters: String = format!("{account_id:x}").bytes().map(|b| CODE_LETTERS[(b as char).to_digit(16).unwrap_or(0) as usize] as char).collect();
    let (a, b) = letters.split_at(letters.len() / 2);
    format!("{a}-{b}")
}

/// The account behind a short code, if it is one.
pub fn account_from_code(code: &str) -> Option<u32> {
    let hex: String = code.chars().filter(|c| *c != '-').map(|c| CODE_LETTERS.iter().position(|l| *l as char == c).map(|i| char::from_digit(i as u32, 16).unwrap_or('0'))).collect::<Option<_>>()?;
    if hex.is_empty() || hex.len() > 8 {
        return None;
    }
    u32::from_str_radix(&hex, 16).ok()
}

/// A Quick Invite link pasted anywhere in `text`: whose it is and its token.
pub fn parse_invite_link(text: &str) -> Option<(SteamId, String)> {
    let i = text.find("s.team/p/")? + "s.team/p/".len();
    let mut parts = text[i..].split(['/', '?', '#', ' ']).filter(|p| !p.is_empty());
    let account = account_from_code(parts.next()?)?;
    let token = parts.next()?.trim().to_string();
    (!token.is_empty()).then_some((STEAMID_BASE + account as u64, token))
}

/// Your Quick Invite link: the newest one still usable, or a new one when
/// there's none (or `new` asks for one).
pub async fn invite(conn: &Connection, me: SteamId, new: bool) -> Result<model::QuickInvite, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let made_by = |t: &CUserAccountCreateFriendInviteTokenResponse| model::QuickInvite {
        // Steam shows the token in lower case; either works.
        link: format!("https://s.team/p/{}/{}", short_code((me & 0xFFFF_FFFF) as u32), t.invite_token().to_lowercase()),
        expires: (t.time_created() as u64 + t.invite_duration()) as u32,
    };
    if !new {
        let r: CUserAccountGetFriendInviteTokensResponse = conn.call("UserAccount.GetFriendInviteTokens#1", &CUserAccountGetFriendInviteTokensRequest {}).await.map_err(|e| e.to_string())?;
        // One-use links only (Steam's default), still valid and not about to run out.
        if let Some(t) = r
            .tokens
            .iter()
            .filter(|t| t.valid() && t.invite_limit() == 1 && !t.invite_token().is_empty() && (t.time_created() as u64 + t.invite_duration()) > now + 3600)
            .max_by_key(|t| t.time_created())
        {
            return Ok(made_by(t));
        }
    }
    let req = CUserAccountCreateFriendInviteTokenRequest { invite_limit: Some(1), invite_duration: Some(INVITE_DAYS * 24 * 3600), invite_note: None };
    let t: CUserAccountCreateFriendInviteTokenResponse = conn.call("UserAccount.CreateFriendInviteToken#1", &req).await.map_err(|e| e.to_string())?;
    if t.invite_token().is_empty() {
        return Err("Steam didn't make a link".into());
    }
    Ok(made_by(&t))
}

/// Use someone's Quick Invite link: you become friends right away.
pub async fn redeem(conn: &Connection, steam_id: SteamId, token: &str) -> Result<(), String> {
    // Links show the token in lower case; Steam issues it in upper case.
    let req = CUserAccountRedeemFriendInviteTokenRequest { steamid: Some(steam_id), invite_token: Some(token.to_uppercase()) };
    let _: CUserAccountRedeemFriendInviteTokenResponse = conn.call("UserAccount.RedeemFriendInviteToken#1", &req).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Profiles whose name matches `text`, from Steam Community's search.
pub async fn search(web: &Web, text: &str, page: u32) -> Result<model::UserSearch, String> {
    let v: Value = web
        .http
        .get("https://steamcommunity.com/search/SearchCommunityAjax")
        .query(&[("text", text), ("filter", "users"), ("sessionid", web.session_id.as_str()), ("steamid_user", "false"), ("page", &page.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|_| "Search isn't available right now".to_string())?;
    if v.get("success").and_then(|s| s.as_i64().or_else(|| s.as_bool().map(i64::from))) != Some(1) {
        return Err("Search isn't available right now".into());
    }
    let html = v.get("html").and_then(|h| h.as_str()).unwrap_or_default();
    let total = v.get("search_result_count").and_then(|n| n.as_u64().or_else(|| n.as_str()?.parse().ok())).unwrap_or(0) as u32;
    let users = parse_search(html);
    tracing::trace!("user search html: {}", &html[..html.len().min(4000)]);
    if users.is_empty() && total > 0 {
        tracing::debug!("user search html: {}", &html[..html.len().min(3000)]);
    }
    Ok(model::UserSearch { text: text.to_string(), page, total, users })
}

/// The result rows of Steam Community's people search.
pub fn parse_search(html: &str) -> Vec<model::FoundUser> {
    html.split("class=\"search_row")
        .skip(1)
        .filter_map(|row| {
            let account: u64 = attr(row, "data-miniprofile")?.parse().ok()?;
            let name_at = row.find("searchPersonaName")?;
            let name = between(&row[name_at..], ">", "</a>")?;
            let profile = attr(&row[name_at..], "href").unwrap_or_default();
            let avatar = row.find("<img").and_then(|i| attr(&row[i..], "src")).unwrap_or_default();
            // Under the name: real name and/or country, and old names ("Also known as").
            let info = row.find("searchPersonaInfo").map(|i| &row[i..]).unwrap_or_default();
            let after_name = info.find("</a>").map(|i| &info[i + 4..]).unwrap_or_default();
            // The Add/Report buttons come after; old names ("Also known as") too.
            let details = text_of(after_name.split("search_result_friend").next().unwrap_or_default().split("search_match_info").next().unwrap_or_default());
            Some(model::FoundUser { steam_id: STEAMID_BASE + account, name: unescape(name.trim()), avatar, profile, details })
        })
        .collect()
}

fn attr(s: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = s.find(&key)? + key.len();
    let j = s[i..].find('"')? + i;
    Some(unescape(&s[i..j]))
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

/// Visible text of an HTML fragment, one line per block, blank lines dropped.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                if tag.starts_with("br") || tag.starts_with("div") || tag.starts_with("/div") {
                    out.push('\n');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    unescape(&out).lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_codes_match_steams() {
        assert_eq!(short_code(836957442), "fcvd-whbd");
        assert_eq!(account_from_code("fcvd-whbd"), Some(836957442));
        assert_eq!(account_from_code("hello"), None);
    }

    #[test]
    fn reads_invite_links() {
        let (id, token) = parse_invite_link("join me: https://s.team/p/fcvd-whbd/kjqgknfr thanks").unwrap();
        assert_eq!(id, STEAMID_BASE + 836957442);
        assert_eq!(token, "kjqgknfr");
        assert!(parse_invite_link("https://s.team/p/fcvd-whbd").is_none());
    }

    #[test]
    fn reads_search_rows() {
        let html = r##"<div class="search_row" data-panel="{}"><div class="mediumHolder_default" data-miniprofile="1234"><div class="avatarMedium"><a href="https://steamcommunity.com/id/ea"><img src="https://avatars.example/a.jpg"></a></div></div><div class="searchPersonaInfo"><a class="searchPersonaName" href="https://steamcommunity.com/id/ea">EA &amp; co</a><br />John Smith<br />Moscow, Russia&nbsp;<img src="flag.gif"></div><div class="search_result_friend"><a href="#" class="btnv6_blue_hoverfade"><span>Add as friend</span></a></div></div>"##;
        let u = parse_search(html);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].steam_id, STEAMID_BASE + 1234);
        assert_eq!(u[0].name, "EA & co");
        assert_eq!(u[0].profile, "https://steamcommunity.com/id/ea");
        assert_eq!(u[0].avatar, "https://avatars.example/a.jpg");
        assert_eq!(u[0].details, "John Smith\nMoscow, Russia");
    }
}

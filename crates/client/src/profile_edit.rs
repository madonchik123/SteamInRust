//! Editing your own profile, the way Steam's "Edit Profile" pages do it:
//! name, real name, custom URL, location and summary, privacy settings,
//! avatar, and the profile items you own (backgrounds, mini profile
//! backgrounds, avatar frames, animated avatars) and themes.
//!
//! The current values come from the edit page's own data; saving uses the
//! same steamcommunity.com calls as the page, and items and themes use the
//! Player service over the Steam connection.

use crate::market::Web;
use model::{ProfileEdit, ProfileItemChoice, ProfilePrivacy};
use net::Connection;
use proto::*;
use serde_json::Value;

const COMMUNITY: &str = "https://steamcommunity.com";

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(|s| s.as_str()).unwrap_or_default().to_string()
}

fn num(v: &Value, k: &str) -> Option<i64> {
    let x = v.get(k)?;
    x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))
}

/// A `data-…="{json}"` attribute of a page, unescaped and parsed.
pub fn data_attr(body: &str, name: &str) -> Option<Value> {
    let start = body.find(&format!("{name}=\""))? + name.len() + 2;
    let end = body[start..].find('"')? + start;
    let raw = body[start..end].replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&amp;", "&");
    serde_json::from_str(&raw).ok()
}

/// Read the edit page's data (`data-profile-edit`).
pub fn parse(body: &str) -> Result<ProfileEdit, String> {
    let d = data_attr(body, "data-profile-edit").ok_or("Steam's edit page didn't have the profile's details")?;
    let loc = d.get("LocationData").cloned().unwrap_or(Value::Null);
    let privacy = d.get("Privacy").cloned().unwrap_or(Value::Null);
    let p = privacy.get("PrivacySettings").cloned().unwrap_or(Value::Null);
    let level = |k: &str| num(&p, k).unwrap_or(3) as u8;
    Ok(ProfileEdit {
        persona_name: text(&d, "strPersonaName"),
        real_name: text(&d, "strRealName"),
        custom_url: text(&d, "strCustomURL"),
        summary: text(&d, "strSummary"),
        country: text(&loc, "locCountryCode"),
        state: text(&loc, "locStateCode"),
        city: num(&loc, "locCityCode").filter(|c| *c > 0).map(|c| c.to_string()).unwrap_or_default(),
        location_text: [text(&loc, "locCity"), text(&loc, "locState"), text(&loc, "locCountry")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
        privacy: ProfilePrivacy {
            profile: level("PrivacyProfile"),
            inventory: level("PrivacyInventory"),
            gifts: level("PrivacyInventoryGifts"),
            games: level("PrivacyOwnedGames"),
            playtime: level("PrivacyPlaytime"),
            friends: level("PrivacyFriendsList"),
            comments: num(&privacy, "eCommentPermission").unwrap_or(1) as u8,
        },
        hide_awards: d.get("ProfilePreferences").and_then(|x| num(x, "hide_profile_awards")).unwrap_or(0) == 1,
        theme: d.get("ActiveTheme").map(|t| text(t, "theme_id")).unwrap_or_default(),
        themes: d
            .get("rgAvailableThemes")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
            .map(|t| (text(t, "theme_id"), text(t, "title")))
            .filter(|t| !t.0.is_empty())
            .collect(),
        ..Default::default()
    })
}

/// Your profile's editable details, privacy, owned items and themes.
pub async fn load(web: &Web, conn: &Connection, language: &str) -> Result<ProfileEdit, String> {
    let body = crate::market::page(web, &format!("{COMMUNITY}/profiles/{}/edit/info", web.me)).await?;
    let mut e = parse(&body)?;
    let owned: Result<CPlayerGetProfileItemsOwnedResponse, _> =
        conn.call("Player.GetProfileItemsOwned#1", &CPlayerGetProfileItemsOwnedRequest { language: Some(language.to_string()), ..Default::default() }).await;
    if let Ok(o) = owned {
        let full = |s: &str| if s.is_empty() { String::new() } else { model::art::community_item(s) };
        let list = |l: &[ProfileItem]| -> Vec<ProfileItemChoice> {
            l.iter()
                .map(|i| ProfileItemChoice {
                    id: i.communityitemid(),
                    name: Some(i.item_title().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| i.name().to_string()),
                    image: Some(i.image_small()).filter(|s| !s.is_empty()).or(Some(i.image_large())).map(model::art::community_item).unwrap_or_default(),
                    app_id: i.appid(),
                    large: full(i.image_large()),
                    movie: full(i.movie_mp4()),
                    app_name: String::new(),
                })
                .collect()
        };
        e.backgrounds = list(&o.profile_backgrounds);
        e.mini_backgrounds = list(&o.mini_profile_backgrounds);
        e.frames = list(&o.avatar_frames);
        e.animated_avatars = list(&o.animated_avatars);
        // Which game each item comes from, by name.
        let mut apps: Vec<u32> = e.backgrounds.iter().chain(&e.mini_backgrounds).chain(&e.frames).chain(&e.animated_avatars).map(|i| i.app_id).filter(|a| *a != 0).collect();
        apps.sort_unstable();
        apps.dedup();
        if let Ok(names) = crate::pics::names(conn, &apps).await {
            let names: std::collections::HashMap<u32, String> = names.into_iter().collect();
            for i in e.backgrounds.iter_mut().chain(e.mini_backgrounds.iter_mut()).chain(e.frames.iter_mut()).chain(e.animated_avatars.iter_mut()) {
                i.app_name = names.get(&i.app_id).cloned().unwrap_or_default();
            }
        }
    }
    let equipped: Result<CPlayerGetProfileItemsEquippedResponse, _> =
        conn.call("Player.GetProfileItemsEquipped#1", &CPlayerGetProfileItemsEquippedRequest { steamid: Some(web.me), language: Some(language.to_string()) }).await;
    if let Ok(q) = equipped {
        let id = |i: &Option<ProfileItem>| i.as_ref().map(|i| i.communityitemid()).unwrap_or(0);
        e.equipped = [id(&q.profile_background), id(&q.mini_profile_background), id(&q.avatar_frame), id(&q.animated_avatar)];
        e.background_full = q.profile_background.as_ref().is_some_and(|i| i.equipped_flags() & 1 != 0);
    }
    let badge: Result<CPlayerGetFavoriteBadgeResponse, _> = conn.call("Player.GetFavoriteBadge#1", &CPlayerGetFavoriteBadgeRequest { steamid: Some(web.me) }).await;
    if let Ok(b) = badge {
        e.favorite_badge = if b.has_favorite_badge() { b.badgeid() } else { 0 };
    }
    Ok(e)
}

/// Save name, real name, custom URL, location and summary.
pub async fn save_info(web: &Web, e: &ProfileEdit) -> Result<(), String> {
    let form = [
        ("sessionID", web.session_id.clone()),
        ("type", "profileSave".into()),
        ("weblink_1_title", String::new()),
        ("weblink_1_url", String::new()),
        ("weblink_2_title", String::new()),
        ("weblink_2_url", String::new()),
        ("weblink_3_title", String::new()),
        ("weblink_3_url", String::new()),
        ("personaName", e.persona_name.clone()),
        ("real_name", e.real_name.clone()),
        ("customURL", e.custom_url.clone()),
        ("country", e.country.clone()),
        ("state", e.state.clone()),
        ("city", e.city.clone()),
        ("summary", e.summary.clone()),
        ("hide_profile_awards", if e.hide_awards { "1" } else { "0" }.into()),
        ("json", "1".into()),
    ];
    let r = web
        .http
        .post(format!("{COMMUNITY}/profiles/{}/edit/", web.me))
        .header("Referer", format!("{COMMUNITY}/profiles/{}/edit/info", web.me))
        .form(&form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    match num(&v, "success") {
        Some(1) => Ok(()),
        _ => Err(Some(text(&v, "errmsg")).filter(|m| !m.is_empty()).map(|m| strip_tags(&m)).unwrap_or_else(|| format!("Steam answered {status}"))),
    }
}

fn strip_tags(s: &str) -> String {
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Save privacy settings (1 private, 2 friends only, 3 public; comments:
/// 0 friends only, 1 public, 2 private).
pub async fn save_privacy(web: &Web, p: &ProfilePrivacy) -> Result<(), String> {
    let privacy = serde_json::json!({
        "PrivacyProfile": p.profile,
        "PrivacyInventory": p.inventory,
        "PrivacyInventoryGifts": p.gifts,
        "PrivacyOwnedGames": p.games,
        "PrivacyPlaytime": p.playtime,
        "PrivacyFriendsList": p.friends,
    });
    let r = web
        .http
        .post(format!("{COMMUNITY}/profiles/{}/ajaxsetprivacy/", web.me))
        .header("Referer", format!("{COMMUNITY}/profiles/{}/edit/settings", web.me))
        .form(&[("sessionid", web.session_id.clone()), ("Privacy", privacy.to_string()), ("eCommentPermission", p.comments.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    match num(&v, "success") {
        Some(1) => Ok(()),
        _ => Err(format!("Steam answered {status}")),
    }
}

/// Upload a picture as your avatar (JPEG or PNG).
pub async fn upload_avatar(web: &Web, bytes: Vec<u8>, file_name: &str) -> Result<(), String> {
    let mime = if file_name.to_lowercase().ends_with(".png") { "image/png" } else { "image/jpeg" };
    let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string()).mime_str(mime).map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .part("avatar", part)
        .text("type", "player_avatar_image")
        .text("sId", web.me.to_string())
        .text("sessionid", web.session_id.clone())
        .text("doSub", "1")
        .text("json", "1");
    let r = web
        .http
        .post(format!("{COMMUNITY}/actions/FileUploader/"))
        .header("Referer", format!("{COMMUNITY}/profiles/{}/edit/avatar", web.me))
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if v.get("success").and_then(|s| s.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(Some(text(&v, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| format!("Steam answered {status}")))
    }
}

/// Which kind of profile item: 0 background, 1 mini profile background,
/// 2 avatar frame, 3 animated avatar. `id` 0 takes it off.
pub async fn equip(conn: &Connection, kind: u8, id: u64) -> Result<(), String> {
    let item = Some(id);
    let r = match kind {
        0 => conn.call::<_, CPlayerSetProfileBackgroundResponse>("Player.SetProfileBackground#1", &CPlayerSetProfileBackgroundRequest { communityitemid: item }).await.map(|_| ()),
        1 => conn.call::<_, CPlayerSetMiniProfileBackgroundResponse>("Player.SetMiniProfileBackground#1", &CPlayerSetMiniProfileBackgroundRequest { communityitemid: item }).await.map(|_| ()),
        2 => conn.call::<_, CPlayerSetAvatarFrameResponse>("Player.SetAvatarFrame#1", &CPlayerSetAvatarFrameRequest { communityitemid: item }).await.map(|_| ()),
        _ => conn.call::<_, CPlayerSetAnimatedAvatarResponse>("Player.SetAnimatedAvatar#1", &CPlayerSetAnimatedAvatarRequest { communityitemid: item }).await.map(|_| ()),
    };
    r.map_err(|e| e.to_string())
}

/// Steam's "Full Screen" (flag 1) or "Original Size" for a background.
pub async fn set_background_full(conn: &Connection, id: u64, full: bool) -> Result<(), String> {
    conn.call::<_, CPlayerSetEquippedProfileItemFlagsResponse>("Player.SetEquippedProfileItemFlags#1", &CPlayerSetEquippedProfileItemFlagsRequest { communityitemid: Some(id), flags: Some(full as u32) })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Feature a community badge on your profile (0 takes it off).
pub async fn set_favorite_badge(conn: &Connection, badge: u32) -> Result<(), String> {
    conn.call::<_, CPlayerSetFavoriteBadgeResponse>("Player.SetFavoriteBadge#1", &CPlayerSetFavoriteBadgeRequest { badgeid: Some(badge), communityitemid: None })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub async fn set_theme(conn: &Connection, theme: &str) -> Result<(), String> {
    conn.call::<_, CPlayerSetProfileThemeResponse>("Player.SetProfileTheme#1", &CPlayerSetProfileThemeRequest { theme_id: Some(theme.to_string()) })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Countries (or a country's states, or a state's cities) for the location
/// pickers: (code, name).
pub async fn locations(web: &Web, country: Option<&str>, state: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let url = match (country, state) {
        (Some(c), Some(s)) => format!("{COMMUNITY}/actions/QueryLocations/{c}/{s}"),
        (Some(c), None) => format!("{COMMUNITY}/actions/QueryLocations/{c}"),
        _ => format!("{COMMUNITY}/actions/QueryLocations/"),
    };
    let v: Value = web.http.get(url).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    Ok(parse_locations(&v))
}

pub fn parse_locations(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let code = ["countrycode", "statecode", "cityid"].iter().find_map(|k| x.get(*k).map(|c| c.as_str().map(String::from).unwrap_or_else(|| c.to_string())))?;
            let name = ["countryname", "statename", "cityname"].iter().find_map(|k| x.get(*k).and_then(|n| n.as_str()))?.to_string();
            Some((code, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_edit_page() {
        let body = r#"<div id="profile_edit_config" data-profile-edit="{&quot;strPersonaName&quot;:&quot;Pal &amp; Co&quot;,&quot;strCustomURL&quot;:&quot;pal&quot;,&quot;strRealName&quot;:&quot;&quot;,&quot;strSummary&quot;:&quot;hi&quot;,&quot;LocationData&quot;:{&quot;locCountry&quot;:&quot;Kazakhstan&quot;,&quot;locCountryCode&quot;:&quot;KZ&quot;,&quot;locState&quot;:&quot;&quot;,&quot;locStateCode&quot;:&quot;&quot;,&quot;locCity&quot;:&quot;&quot;,&quot;locCityCode&quot;:0},&quot;Privacy&quot;:{&quot;PrivacySettings&quot;:{&quot;PrivacyProfile&quot;:3,&quot;PrivacyInventory&quot;:2,&quot;PrivacyInventoryGifts&quot;:1,&quot;PrivacyOwnedGames&quot;:3,&quot;PrivacyPlaytime&quot;:3,&quot;PrivacyFriendsList&quot;:3},&quot;eCommentPermission&quot;:0},&quot;ProfilePreferences&quot;:{&quot;hide_profile_awards&quot;:1}}"></div>"#;
        let e = super::parse(body).unwrap();
        assert_eq!((e.persona_name.as_str(), e.custom_url.as_str(), e.country.as_str(), e.location_text.as_str()), ("Pal & Co", "pal", "KZ", "Kazakhstan"));
        assert_eq!((e.privacy.inventory, e.privacy.gifts, e.privacy.comments), (2, 1, 0));
        assert!(e.hide_awards);
        assert!(e.city.is_empty());
        let l: serde_json::Value = serde_json::from_str(r#"[{"countrycode":"KZ","hasstates":1,"countryname":"Kazakhstan"},{"statecode":"02","statename":"Almaty"},{"cityid":1234,"cityname":"Almaty"}]"#).unwrap();
        assert_eq!(super::parse_locations(&l), vec![("KZ".into(), "Kazakhstan".into()), ("02".into(), "Almaty".into()), ("1234".into(), "Almaty".into())]);
    }
}

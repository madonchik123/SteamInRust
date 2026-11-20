//! Chat beyond plain messages, with the services Steam's own chat uses:
//! reactions, reports, your emoticons/stickers/effects, pictures, your trade
//! link, and making, joining and leaving group chats.

use crate::ChatTarget;
use crate::market::Web;
use model::{Emoticons, OwnedEmote, Reaction, SteamId};
use net::Connection;
use proto::*;

const INDIVIDUAL: u64 = 76561197960265728;
const COMMUNITY: &str = "https://steamcommunity.com";

/// Steam's reaction type: 1 an emoticon, 2 a sticker.
fn reaction_type(sticker: bool) -> i32 {
    if sticker { 2 } else { 1 }
}

/// Add or take back a reaction. Answers whether Steam took it.
pub async fn react(conn: &Connection, to: ChatTarget, time: u32, ordinal: u32, reaction: &str, sticker: bool, add: bool) -> Result<(), String> {
    match to {
        ChatTarget::Friend(friend) => {
            let _: CFriendMessagesUpdateMessageReactionResponse = conn
                .call(
                    "FriendMessages.UpdateMessageReaction#1",
                    &CFriendMessagesUpdateMessageReactionRequest {
                        steamid: Some(friend),
                        server_timestamp: Some(time),
                        ordinal: Some(ordinal),
                        reaction_type: Some(reaction_type(sticker)),
                        reaction: Some(reaction.to_string()),
                        is_add: Some(add),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        ChatTarget::Group { group, room } => {
            let _: CChatRoomUpdateMessageReactionResponse = conn
                .call(
                    "ChatRoom.UpdateMessageReaction#1",
                    &CChatRoomUpdateMessageReactionRequest {
                        chat_group_id: Some(group),
                        chat_id: Some(room),
                        server_timestamp: Some(time),
                        ordinal: Some(ordinal),
                        reaction_type: Some(reaction_type(sticker)),
                        reaction: Some(reaction.to_string()),
                        is_add: Some(add),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Report a message to Steam's moderators. `reason` is Steam's
/// EContentReportReason.
#[allow(clippy::too_many_arguments)]
pub async fn report(conn: &Connection, me: SteamId, to: ChatTarget, from: SteamId, time: u32, ordinal: u32, reason: i32, text: &str, language: &str) -> Result<(), String> {
    match to {
        ChatTarget::Friend(friend) => {
            let _: CFriendMessagesReportMessageResponse = conn
                .call(
                    "FriendMessages.ReportMessage#1",
                    &CFriendMessagesReportMessageRequest {
                        steamid_from: Some(from),
                        steamid_to: Some(if from == friend { me } else { friend }),
                        timestamp: Some(time),
                        ordinal: Some(ordinal),
                        report_reason: Some(reason),
                        report_text: Some(text.to_string()),
                        language: Some(language.to_string()),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        ChatTarget::Group { group, room } => {
            let _: CChatRoomReportMessageResponse = conn
                .call(
                    "ChatRoom.ReportMessage#1",
                    &CChatRoomReportMessageRequest {
                        chat_group_id: Some(group),
                        chat_id: Some(room),
                        steamid_from: Some(from),
                        timestamp: Some(time),
                        ordinal: Some(ordinal),
                        report_reason: Some(reason),
                        report_text: Some(text.to_string()),
                        language: Some(language.to_string()),
                        subject_type: Some(5),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Your emoticons, stickers and effects from Steam's emoticon list, the
/// most used first.
pub fn emoticons(list: &CMsgClientEmoticonList) -> Emoticons {
    let mut e = Emoticons {
        emoticons: list.emoticons.iter().map(|x| OwnedEmote { name: x.name().trim_matches(':').to_string(), app_id: x.appid(), last_used: x.time_last_used(), use_count: x.use_count() }).collect(),
        stickers: list.stickers.iter().map(|x| OwnedEmote { name: x.name().to_string(), app_id: x.appid(), last_used: x.time_last_used(), use_count: x.use_count() }).collect(),
        effects: list.effects.iter().map(|x| OwnedEmote { name: x.name().to_string(), app_id: x.appid(), ..Default::default() }).collect(),
    };
    for l in [&mut e.emoticons, &mut e.stickers] {
        l.retain(|x| !x.name.is_empty());
        l.sort_by(|a, b| b.use_count.cmp(&a.use_count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }
    e.effects.retain(|x| !x.name.is_empty());
    e
}

/// Reactions on a one-to-one message (who reacted, by account id).
pub fn friend_reactions(list: &[c_friend_messages_get_recent_messages_response::friend_message::MessageReaction], me: SteamId) -> Vec<Reaction> {
    let mine = (me & 0xFFFF_FFFF) as u32;
    list.iter()
        .filter(|r| !r.reaction().is_empty() && !r.reactors.is_empty())
        .map(|r| Reaction { name: r.reaction().to_string(), sticker: r.reaction_type() as i32 == 2, count: r.reactors.len() as u32, mine: r.reactors.contains(&mine) })
        .collect()
}

/// Reactions on a group chat message.
pub fn group_reactions(list: &[c_chat_room_get_message_history_response::chat_message::MessageReaction]) -> Vec<Reaction> {
    list.iter()
        .filter(|r| !r.reaction().is_empty() && r.num_reactors() > 0)
        .map(|r| Reaction { name: r.reaction().to_string(), sticker: r.reaction_type() as i32 == 2, count: r.num_reactors(), mine: r.has_user_reacted() })
        .collect()
}

/// Your trade offer link (`new` makes a new one, which stops the old one
/// working).
pub async fn trade_link(conn: &Connection, me: SteamId, new: bool) -> Result<String, String> {
    let r: CEconGetTradeOfferAccessTokenResponse =
        conn.call("Econ.GetTradeOfferAccessToken#1", &CEconGetTradeOfferAccessTokenRequest { generate_new_token: Some(new) }).await.map_err(|e| e.to_string())?;
    let token = r.trade_offer_access_token();
    if token.is_empty() {
        return Err("Steam didn't give a trade link".into());
    }
    Ok(format!("{COMMUNITY}/tradeoffer/new/?partner={}&token={token}", me.saturating_sub(INDIVIDUAL)))
}

/// A new group chat. With a `partner`, it's the one-to-one chat with them
/// grown into a group (Steam's "invite a friend to this chat"); `name` empty
/// makes an unnamed one. Answers its id.
pub async fn create_group(conn: &Connection, name: &str, partner: Option<SteamId>, invitees: &[SteamId]) -> Result<u64, String> {
    let req = CChatRoomCreateChatRoomGroupRequest {
        steamid_partner: partner,
        steamid_invited: partner.and(invitees.first().copied()),
        name: Some(name.to_string()),
        steamid_invitees: if partner.is_some() { invitees.iter().skip(1).copied().collect() } else { invitees.to_vec() },
        ..Default::default()
    };
    let r: CChatRoomCreateChatRoomGroupResponse = conn.call("ChatRoom.CreateChatRoomGroup#1", &req).await.map_err(|e| e.to_string())?;
    Ok(r.chat_group_id())
}

pub async fn invite(conn: &Connection, group: u64, friend: SteamId) -> Result<(), String> {
    let _: CChatRoomInviteFriendToChatRoomGroupResponse = conn
        .call("ChatRoom.InviteFriendToChatRoomGroup#1", &CChatRoomInviteFriendToChatRoomGroupRequest { chat_group_id: Some(group), steamid: Some(friend), ..Default::default() })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn leave(conn: &Connection, group: u64) -> Result<(), String> {
    let _: CChatRoomLeaveChatRoomGroupResponse =
        conn.call("ChatRoom.LeaveChatRoomGroup#1", &CChatRoomLeaveChatRoomGroupRequest { chat_group_id: Some(group) }).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// What a picture is, for the upload.
fn mime(bytes: &[u8]) -> Option<&'static str> {
    Some(match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => return None,
    })
}

/// The first string named `key` anywhere in a JSON answer.
fn find_str(v: &serde_json::Value, key: &str) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => m.get(key).and_then(|x| x.as_str()).map(String::from).or_else(|| m.values().find_map(|x| find_str(x, key))),
        serde_json::Value::Array(a) => a.iter().find_map(|x| find_str(x, key)),
        _ => None,
    }
}

/// Upload a picture for a chat the way Steam's chat does (ask for an
/// upload slot, put the file there, confirm it), and answer its address
/// and size, for the `[img]` message.
///
/// Not tried against Steam yet (testing it sends a real picture): if Steam
/// changed a field, this fails with Steam's answer rather than sending
/// anything half-done.
pub async fn upload_image(web: &Web, to: ChatTarget, bytes: Vec<u8>, file_name: &str) -> Result<(String, u32, u32), String> {
    use sha1::Digest;
    let mime = mime(&bytes).ok_or("That file isn't a picture Steam's chat takes (PNG, JPEG, GIF or WebP)")?;
    let (w, h) = image::load_from_memory(&bytes).map(|i| (i.width(), i.height())).map_err(|e| format!("Couldn't read the picture: {e}"))?;
    let sha: String = sha1::Sha1::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let size = bytes.len().to_string();
    let (w_s, h_s) = (w.to_string(), h.to_string());
    let begin: serde_json::Value = web
        .http
        .post(format!("{COMMUNITY}/chat/beginfileupload/?l=english"))
        .form(&[
            ("sessionid", web.session_id.as_str()),
            ("l", "english"),
            ("file_size", &size),
            ("file_name", file_name),
            ("file_sha", &sha),
            ("file_image_width", &w_s),
            ("file_image_height", &h_s),
            ("file_type", mime),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| format!("Steam's upload answer: {e}"))?;
    let r = begin.get("result").ok_or_else(|| format!("Steam didn't open an upload: {begin}"))?;
    let get = |k: &str| r.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).filter(|s| !s.is_empty() && s != "null");
    let (Some(ugcid), Some(timestamp), Some(hmac), Some(host), Some(path)) = (get("ugcid"), get("timestamp"), get("hmac"), get("url_host"), get("url_path")) else {
        return Err(format!("Steam's upload answer is missing parts: {r}"));
    };
    let https = r.get("use_https").and_then(|v| v.as_bool()).unwrap_or(true);
    let mut put = web.http.put(format!("{}://{host}{path}", if https { "https" } else { "http" })).body(bytes);
    for hd in r.get("request_headers").and_then(|v| v.as_array()).into_iter().flatten() {
        let (Some(n), Some(v)) = (hd.get("name").and_then(|x| x.as_str()), hd.get("value").and_then(|x| x.as_str())) else { continue };
        if !n.eq_ignore_ascii_case("content-length") {
            put = put.header(n, v);
        }
    }
    let status = put.send().await.map_err(|e| e.to_string())?.status();
    if !status.is_success() {
        return Err(format!("The picture didn't upload ({status})"));
    }
    let (friend, group, room) = match to {
        ChatTarget::Friend(f) => (f.to_string(), String::new(), String::new()),
        ChatTarget::Group { group, room } => (String::new(), group.to_string(), room.to_string()),
    };
    let commit: serde_json::Value = web
        .http
        .post(format!("{COMMUNITY}/chat/commitfileupload/"))
        .form(&[
            ("sessionid", web.session_id.as_str()),
            ("l", "english"),
            ("file_name", file_name),
            ("file_sha", &sha),
            ("success", "1"),
            ("ugcid", &ugcid),
            ("file_type", mime),
            ("time", &timestamp),
            ("hmac", &hmac),
            ("friend_steamid", &friend),
            ("chat_group_id", &group),
            ("chat_id", &room),
            ("spoiler", "0"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| format!("Steam's answer to the upload: {e}"))?;
    if commit.get("success").and_then(|v| v.as_i64()).is_some_and(|s| s != 1) {
        return Err(format!("Steam didn't take the picture: {commit}"));
    }
    let url = find_str(&commit, "url").unwrap_or_else(|| format!("https://images.steamusercontent.com/ugc/{ugcid}/{}/", sha.to_uppercase()));
    Ok((url, w, h))
}

/// Your newest screenshot (Steam keeps them in
/// `userdata/<account>/760/remote/<game>/screenshots`).
pub fn newest_screenshot(steam_root: &std::path::Path, me: SteamId) -> Option<std::path::PathBuf> {
    let base = steam_root.join("userdata").join((me & 0xFFFF_FFFF).to_string()).join("760").join("remote");
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for game in std::fs::read_dir(base).ok()?.flatten() {
        let Ok(files) = std::fs::read_dir(game.path().join("screenshots")) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("png")) {
                continue;
            }
            let Ok(t) = f.metadata().and_then(|m| m.modified()) else { continue };
            if best.as_ref().is_none_or(|(bt, _)| t > *bt) {
                best = Some((t, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoticon_list_most_used_first() {
        use c_msg_client_emoticon_list::{Effect, Emoticon, Sticker};
        let list = CMsgClientEmoticonList {
            emoticons: vec![
                Emoticon { name: Some(":steamsad:".into()), use_count: Some(1), appid: Some(753), ..Default::default() },
                Emoticon { name: Some(":steamhappy:".into()), use_count: Some(9), ..Default::default() },
            ],
            stickers: vec![Sticker { name: Some("Sticker_Hi".into()), ..Default::default() }],
            effects: vec![Effect { name: Some("confetti".into()), infinite_use: Some(true), ..Default::default() }],
        };
        let e = emoticons(&list);
        assert_eq!(e.emoticons.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["steamhappy", "steamsad"]);
        assert_eq!(e.emoticons[1].app_id, 753);
        assert_eq!(e.stickers[0].name, "Sticker_Hi");
        assert_eq!(e.effects[0].name, "confetti");
    }

    #[test]
    fn reactions_from_history() {
        use c_friend_messages_get_recent_messages_response::friend_message::MessageReaction;
        let me = INDIVIDUAL + 5;
        let r = friend_reactions(&[MessageReaction { reaction_type: Some(1), reaction: Some("steamhappy".into()), reactors: vec![5, 9] }, MessageReaction { reaction_type: Some(2), reaction: Some("x".into()), reactors: vec![] }], me);
        assert_eq!(r, vec![Reaction { name: "steamhappy".into(), sticker: false, count: 2, mine: true }]);
    }

    #[test]
    fn finds_nested_urls() {
        let v: serde_json::Value = serde_json::from_str(r#"{"success":1,"result":{"details":{"url":"https://images.example/u/1/"}}}"#).unwrap();
        assert_eq!(find_str(&v, "url").as_deref(), Some("https://images.example/u/1/"));
    }
}

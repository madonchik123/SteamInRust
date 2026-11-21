//! Steam group chats (the chat rooms in Steam's Friends & Chat): your
//! groups and their text channels, message history, sending, and the live
//! messages Steam pushes once the groups are marked active for this session.

use model::{ChatGroup, ChatRoom, GroupMessage, SteamId};
use net::Connection;
use proto::*;

/// Steam ids of individual accounts are this plus the account id.
const INDIVIDUAL: u64 = 76561197960265728;

fn sha_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn avatar(ugc: &str, sha: &[u8]) -> String {
    if !ugc.is_empty() {
        ugc.to_string()
    } else if sha.iter().any(|b| *b != 0) {
        format!("https://avatars.fastly.steamstatic.com/{}_full.jpg", sha_hex(sha))
    } else {
        String::new()
    }
}

fn room(r: &CChatRoomState, read_up_to: u32) -> ChatRoom {
    ChatRoom {
        id: r.chat_id(),
        name: r.chat_name().to_string(),
        last_message: r.last_message().to_string(),
        last_time: r.time_last_message(),
        last_sender: if r.accountid_last_message() == 0 { 0 } else { INDIVIDUAL + r.accountid_last_message() as u64 },
        unread: r.time_last_message() > read_up_to,
        voice: r.voice_allowed(),
        sort: r.sort_order(),
        in_voice: r.members_in_voice.iter().map(|a| INDIVIDUAL + *a as u64).collect(),
    }
}

fn group(pair: &CChatRoomSummaryPair) -> Option<ChatGroup> {
    let g = pair.group_summary.as_ref()?;
    let user = pair.user_chat_group_state.as_ref();
    // When each room was last read (0 = never).
    let read = |id: u64| user.and_then(|u| u.user_chat_room_state.iter().find(|s| s.chat_id() == id)).map(|s| s.time_last_ack()).unwrap_or(0);
    let mut rooms: Vec<ChatRoom> = g.chat_rooms.iter().map(|r| room(r, read(r.chat_id()))).collect();
    rooms.sort_by_key(|r| (r.sort, r.id));
    let muted = user.is_some_and(|u| u.unread_indicator_muted());
    Some(ChatGroup {
        id: g.chat_group_id(),
        name: g.chat_group_name().to_string(),
        tagline: g.chat_group_tagline().to_string(),
        avatar: avatar(g.avatar_ugc_url(), g.chat_group_avatar_sha()),
        clan_id: g.clanid(),
        app_id: g.appid(),
        default_room: g.default_chat_id(),
        active_members: g.active_member_count(),
        unread: !muted && rooms.iter().any(|r| r.unread),
        muted,
        rooms,
    })
}

/// Your group chats, with their rooms; also marks them active for this
/// session so Steam sends their messages as they arrive. Returns each
/// group's members (who's in it).
pub async fn groups(conn: &Connection) -> Result<(Vec<ChatGroup>, Vec<(u64, Vec<SteamId>)>), String> {
    let r: CChatRoomGetMyChatRoomGroupsResponse = conn.call("ChatRoom.GetMyChatRoomGroups#1", &CChatRoomGetMyChatRoomGroupsRequest {}).await.map_err(|e| e.to_string())?;
    let mut list: Vec<ChatGroup> = r.chat_room_groups.iter().filter_map(group).collect();
    list.sort_by(|a, b| b.unread.cmp(&a.unread).then(b.rooms.iter().map(|r| r.last_time).max().cmp(&a.rooms.iter().map(|r| r.last_time).max())));
    let ids: Vec<u64> = list.iter().map(|g| g.id).collect();
    let mut members = Vec::new();
    if !ids.is_empty() {
        let active: Result<CChatRoomSetSessionActiveChatRoomGroupsResponse, _> = conn
            .call(
                "ChatRoom.SetSessionActiveChatRoomGroups#1",
                &CChatRoomSetSessionActiveChatRoomGroupsRequest { chat_group_ids: ids.clone(), chat_groups_data_requested: ids, virtualize_members_threshold: Some(1000) },
            )
            .await;
        if let Ok(a) = active {
            for st in &a.chat_states {
                let Some(id) = st.header_state.as_ref().map(|h| h.chat_group_id()) else { continue };
                let who: Vec<SteamId> = st.members.iter().filter(|m| m.state() == EChatRoomJoinState::KEChatRoomJoinStateJoined).map(|m| INDIVIDUAL + m.accountid() as u64).collect();
                members.push((id, who));
            }
        }
    }
    Ok((list, members))
}

/// Messages in a room, newest last; `before` pages back from a message.
/// Returns them and whether there are older ones.
pub async fn history(conn: &Connection, group: u64, room: u64, before: Option<(u32, u32)>) -> Result<(Vec<GroupMessage>, bool), String> {
    let req = CChatRoomGetMessageHistoryRequest {
        chat_group_id: Some(group),
        chat_id: Some(room),
        last_time: before.map(|b| b.0),
        last_ordinal: before.map(|b| b.1),
        max_count: Some(50),
        ..Default::default()
    };
    let r: CChatRoomGetMessageHistoryResponse = conn.call("ChatRoom.GetMessageHistory#1", &req).await.map_err(|e| e.to_string())?;
    let mut out: Vec<GroupMessage> = r
        .messages
        .iter()
        .filter(|m| !m.deleted() && m.server_message.is_none())
        .map(|m| GroupMessage { group, room, sender: INDIVIDUAL + m.sender() as u64, text: m.message().to_string(), time: m.server_timestamp(), ordinal: m.ordinal(), reactions: crate::chat::group_reactions(&m.reactions) })
        .collect();
    out.sort_by_key(|m| (m.time, m.ordinal));
    Ok((out, r.more_available()))
}

/// Send a message; Steam echoes it back as a live message too.
pub async fn send(conn: &Connection, me: SteamId, group: u64, room: u64, text: &str) -> Result<GroupMessage, String> {
    let r: CChatRoomSendChatMessageResponse = conn
        .call("ChatRoom.SendChatMessage#1", &CChatRoomSendChatMessageRequest { chat_group_id: Some(group), chat_id: Some(room), message: Some(text.to_string()), echo_to_sender: Some(true) })
        .await
        .map_err(|e| e.to_string())?;
    let sent = if r.modified_message().is_empty() { text.to_string() } else { r.modified_message().to_string() };
    Ok(GroupMessage { group, room, sender: me, text: sent, time: r.server_timestamp(), ordinal: r.ordinal(), reactions: Vec::new() })
}

/// Mark a room read up to `time`.
pub fn ack(conn: &Connection, group: u64, room: u64, time: u32) {
    let _ = conn.notify_service("ChatRoom.AckChatMessage#1", &CChatRoomAckChatMessageNotification { chat_group_id: Some(group), chat_id: Some(room), timestamp: Some(time) });
}

/// A live message pushed by Steam.
pub fn incoming(n: &CChatRoomIncomingChatMessageNotification) -> Option<GroupMessage> {
    (n.server_message.is_none() && !n.message().is_empty()).then(|| GroupMessage {
        group: n.chat_group_id(),
        room: n.chat_id(),
        sender: n.steamid_sender(),
        text: n.message().to_string(),
        time: n.timestamp(),
        ordinal: n.ordinal(),
        reactions: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_groups_rooms_and_unread() {
        let pair = CChatRoomSummaryPair {
            user_chat_group_state: Some(CUserChatRoomGroupState {
                chat_group_id: Some(7),
                user_chat_room_state: vec![CUserChatRoomState { chat_id: Some(70), time_last_ack: Some(100), ..Default::default() }, CUserChatRoomState { chat_id: Some(71), time_last_ack: Some(500), ..Default::default() }],
                ..Default::default()
            }),
            group_summary: Some(CChatRoomGetChatRoomGroupSummaryResponse {
                chat_group_id: Some(7),
                chat_group_name: Some("Pals".into()),
                chat_group_avatar_sha: Some(vec![0xab; 20]),
                default_chat_id: Some(70),
                chat_rooms: vec![
                    CChatRoomState { chat_id: Some(71), chat_name: Some("memes".into()), time_last_message: Some(400), sort_order: Some(2), ..Default::default() },
                    CChatRoomState { chat_id: Some(70), chat_name: Some("general".into()), time_last_message: Some(200), accountid_last_message: Some(42), sort_order: Some(1), ..Default::default() },
                ],
                ..Default::default()
            }),
        };
        let g = group(&pair).unwrap();
        assert_eq!((g.id, g.name.as_str(), g.default_room), (7, "Pals", 70));
        assert!(g.avatar.ends_with("abababababababababababababababababababab_full.jpg"));
        assert_eq!(g.rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["general", "memes"]);
        assert!(g.rooms[0].unread, "a message after the last read");
        assert!(!g.rooms[1].unread);
        assert_eq!(g.rooms[0].last_sender, INDIVIDUAL + 42);
        assert!(g.unread);
    }
}

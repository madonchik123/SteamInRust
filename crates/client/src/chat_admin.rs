//! Managing group chats, with the `ChatRoom.*` service calls Steam's own
//! Friends & Chat makes: notifications, invite links and invites, the
//! group's name, tagline and icon, channels, roles and what they allow,
//! members' roles, kicks and bans, leaving, and making a new group chat.

use crate::market::Web;
use crate::{Event, Sink};
use model::chat_admin::{AdminEvent, AdminOp, GroupAdmin, INDIVIDUAL, InviteLink, Listed, Member, Notify, Role, RolePerms};
use model::{ChatGroup, ChatRoom, SteamId};
use net::Connection;
use proto::*;
use std::sync::Arc;

const COMMUNITY: &str = "https://steamcommunity.com";

fn person(account: u32) -> SteamId {
    if account == 0 { 0 } else { INDIVIDUAL + account as u64 }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Steam's role toggles from ours.
pub fn actions(role: u64, p: &RolePerms) -> CChatRoleActions {
    CChatRoleActions {
        role_id: Some(role),
        can_create_rename_delete_channel: Some(p.channels),
        can_kick: Some(p.kick),
        can_ban: Some(p.ban),
        can_invite: Some(p.invite),
        can_change_tagline_avatar_name: Some(p.change_details),
        can_chat: Some(p.chat),
        can_view_history: Some(p.view_history),
        can_change_group_roles: Some(p.edit_roles),
        can_change_user_roles: Some(p.assign_roles),
        can_mention_all: Some(p.mention_all),
        can_set_watching_broadcast: Some(p.broadcasts),
    }
}

pub fn perms(a: &CChatRoleActions) -> RolePerms {
    RolePerms {
        chat: a.can_chat(),
        mention_all: a.can_mention_all(),
        broadcasts: a.can_set_watching_broadcast(),
        kick: a.can_kick(),
        ban: a.can_ban(),
        invite: a.can_invite(),
        edit_roles: a.can_change_group_roles(),
        assign_roles: a.can_change_user_roles(),
        change_details: a.can_change_tagline_avatar_name(),
        channels: a.can_create_rename_delete_channel(),
        view_history: a.can_view_history(),
    }
}

/// A group's roles, owner and members' roles (from Steam's group state),
/// with your own settings for it.
pub fn parse(group: u64, st: &CChatRoomGroupState, user: Option<&CUserChatRoomGroupState>) -> GroupAdmin {
    let h = st.header_state.clone().unwrap_or_default();
    let roles = h
        .roles
        .iter()
        .map(|r| Role {
            id: r.role_id(),
            name: r.name().to_string(),
            ordinal: r.ordinal(),
            perms: h.role_actions.iter().find(|a| a.role_id() == r.role_id()).map(perms).unwrap_or_default(),
        })
        .collect();
    let default_role = h.default_role_id();
    let members = st
        .members
        .iter()
        .filter(|m| m.state() == EChatRoomJoinState::KEChatRoomJoinStateJoined)
        .map(|m| Member { steam_id: person(m.accountid()), roles: m.role_ids.iter().copied().filter(|r| *r != default_role).collect() })
        .collect();
    let (notify, room_notify, dms_allowed) = match user {
        Some(u) => (
            Notify::from_level(u.desktop_notification_level() as i32),
            u.user_chat_room_state
                .iter()
                .map(|r| (r.chat_id(), Notify::from_level(r.desktop_notification_level() as i32)))
                .filter(|(_, n)| *n != Notify::Inherit)
                .collect(),
            u.direct_messages_allowed(),
        ),
        None => (Notify::Inherit, vec![], false),
    };
    GroupAdmin {
        group,
        owner: person(h.accountid_owner()),
        default_role,
        roles,
        members,
        // Steam's default when you've never picked: every message.
        notify: if notify == Notify::Inherit { Notify::All } else { notify },
        room_notify,
        dms_allowed,
        ..Default::default()
    }
}

/// A group chat from Steam's state of it (a new group).
pub fn group_from_state(id: u64, st: &CChatRoomGroupState) -> ChatGroup {
    let h = st.header_state.clone().unwrap_or_default();
    let mut rooms: Vec<ChatRoom> = st.chat_rooms.iter().map(|r| ChatRoom { id: r.chat_id(), name: r.chat_name().to_string(), voice: r.voice_allowed(), sort: r.sort_order(), ..Default::default() }).collect();
    rooms.sort_by_key(|r| (r.sort, r.id));
    let sha = h.avatar_sha();
    ChatGroup {
        id,
        name: h.chat_name().to_string(),
        tagline: h.tagline().to_string(),
        avatar: if !h.avatar_ugc_url().is_empty() {
            h.avatar_ugc_url().to_string()
        } else if sha.iter().any(|b| *b != 0) {
            format!("https://avatars.fastly.steamstatic.com/{}_full.jpg", hex(sha))
        } else {
            String::new()
        },
        clan_id: h.clanid(),
        app_id: h.appid(),
        default_room: st.default_chat_id(),
        active_members: 1,
        rooms,
        ..Default::default()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Your notification or direct message setting for a group or a channel.
pub fn prefs_request(group: u64, op: &AdminOp) -> Option<CChatRoomSetUserChatGroupPreferencesRequest> {
    use c_chat_room_set_user_chat_group_preferences_request::{ChatGroupPreferences, ChatRoomPreferences};
    let mut req = CChatRoomSetUserChatGroupPreferencesRequest { chat_group_id: Some(group), ..Default::default() };
    match op {
        AdminOp::SetNotify(n) => req.chat_group_preferences = Some(ChatGroupPreferences { desktop_notification_level: Some(n.level()), ..Default::default() }),
        AdminOp::SetDmsAllowed(on) => req.chat_group_preferences = Some(ChatGroupPreferences { direct_messages_allowed: Some(*on), ..Default::default() }),
        AdminOp::SetRoomNotify { room, level } => req.chat_room_preferences = vec![ChatRoomPreferences { chat_id: Some(*room), desktop_notification_level: Some(level.level()), ..Default::default() }],
        _ => return None,
    }
    Some(req)
}

/// `seconds` `None` (never) is Steam's 0.
pub fn link_request(group: u64, seconds: Option<u32>, room: u64) -> CChatRoomCreateInviteLinkRequest {
    CChatRoomCreateInviteLinkRequest { chat_group_id: Some(group), seconds_valid: Some(seconds.unwrap_or(0)), chat_id: (room != 0).then_some(room) }
}

/// Everything the settings dialog shows. Lists you aren't allowed to see
/// (links, invites, bans) come back empty.
pub async fn load(conn: &Connection, group: u64) -> Result<GroupAdmin, String> {
    let st: CChatRoomGetChatRoomGroupStateResponse = conn.call("ChatRoom.GetChatRoomGroupState#1", &CChatRoomGetChatRoomGroupStateRequest { chat_group_id: Some(group) }).await.map_err(err)?;
    let mine: Option<CChatRoomGetMyChatRoomGroupsResponse> = conn.call("ChatRoom.GetMyChatRoomGroups#1", &CChatRoomGetMyChatRoomGroupsRequest {}).await.ok();
    let user = mine.as_ref().and_then(|m| m.chat_room_groups.iter().find_map(|p| p.user_chat_group_state.as_ref().filter(|u| u.chat_group_id() == group)));
    let mut a = parse(group, &st.state.unwrap_or_default(), user);
    if let Ok(r) = conn.call::<_, CChatRoomGetInviteLinksForGroupResponse>("ChatRoom.GetInviteLinksForGroup#1", &CChatRoomGetInviteLinksForGroupRequest { chat_group_id: Some(group) }).await {
        a.links = r.invite_links.iter().map(|l| InviteLink { code: l.invite_code().to_string(), creator: l.steamid_creator(), expires: l.time_expires(), room: l.chat_id() }).collect();
    }
    if let Ok(r) = conn.call::<_, CChatRoomGetInviteListResponse>("ChatRoom.GetInviteList#1", &CChatRoomGetInviteListRequest { chat_group_id: Some(group) }).await {
        a.invited = r.invites.iter().map(|i| Listed { steam_id: person(i.accountid()), by: person(i.accountid_actor()), time: i.time_invited(), reason: String::new() }).collect();
    }
    if let Ok(r) = conn.call::<_, CChatRoomGetBanListResponse>("ChatRoom.GetBanList#1", &CChatRoomGetBanListRequest { chat_group_id: Some(group) }).await {
        a.bans = r.bans.iter().map(|b| Listed { steam_id: person(b.accountid()), by: person(b.accountid_actor()), time: b.time_banned(), reason: b.ban_reason().to_string() }).collect();
    }
    Ok(a)
}

macro_rules! call {
    ($conn:expr, $method:literal, $resp:ty, $req:expr) => {
        $conn.call::<_, $resp>(concat!("ChatRoom.", $method, "#1"), &$req).await.map(|_| ()).map_err(err)
    };
}

fn now() -> i32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i32).unwrap_or(0)
}

/// Do one change and say how it went (and what it made).
pub async fn run(conn: Connection, web: Option<Result<Arc<Web>, String>>, me: SteamId, group: u64, op: AdminOp, sink: Sink) {
    let g = Some(group);
    let result: Result<(), String> = match &op {
        AdminOp::Load => {
            sink(Event::ChatAdmin(AdminEvent::Loaded { group, result: load(&conn, group).await }));
            return;
        }
        AdminOp::SetNotify(_) | AdminOp::SetRoomNotify { .. } | AdminOp::SetDmsAllowed(_) => match prefs_request(group, &op) {
            Some(req) => call!(conn, "SetUserChatGroupPreferences", CChatRoomSetUserChatGroupPreferencesResponse, req),
            None => Ok(()),
        },
        AdminOp::CreateInviteLink { seconds, room } => {
            match conn.call::<_, CChatRoomCreateInviteLinkResponse>("ChatRoom.CreateInviteLink#1", &link_request(group, *seconds, *room)).await {
                Ok(r) => {
                    let expires = if r.seconds_valid() == 0 { 0 } else { now() as u32 + r.seconds_valid() };
                    sink(Event::ChatAdmin(AdminEvent::LinkCreated { group, link: InviteLink { code: r.invite_code().to_string(), creator: me, expires, room: *room } }));
                    Ok(())
                }
                Err(e) => Err(err(e)),
            }
        }
        AdminOp::DeleteInviteLink { code } => call!(conn, "DeleteInviteLink", CChatRoomDeleteInviteLinkResponse, CChatRoomDeleteInviteLinkRequest { chat_group_id: g, invite_code: Some(code.clone()) }),
        AdminOp::InviteFriend { steam_id, room } => call!(
            conn,
            "InviteFriendToChatRoomGroup",
            CChatRoomInviteFriendToChatRoomGroupResponse,
            CChatRoomInviteFriendToChatRoomGroupRequest { chat_group_id: g, steamid: Some(*steam_id), chat_id: (*room != 0).then_some(*room), skip_friendsui_check: Some(true) }
        ),
        AdminOp::RevokeInvite { steam_id } => call!(conn, "RevokeInviteToGroup", CChatRoomRevokeInviteResponse, CChatRoomRevokeInviteRequest { chat_group_id: g, steamid: Some(*steam_id) }),
        AdminOp::Rename { name } => call!(conn, "RenameChatRoomGroup", CChatRoomRenameChatRoomGroupResponse, CChatRoomRenameChatRoomGroupRequest { chat_group_id: g, name: Some(name.clone()) }),
        AdminOp::SetTagline { tagline } => call!(conn, "SetChatRoomGroupTagline", CChatRoomSetChatRoomGroupTaglineResponse, CChatRoomSetChatRoomGroupTaglineRequest { chat_group_id: g, tagline: Some(tagline.clone()) }),
        AdminOp::SetAvatar { path } => match web {
            Some(Ok(w)) => match std::fs::read(path) {
                Ok(bytes) => match upload_image(&w, &bytes, &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "icon.png".into())).await {
                    Ok(sha) => call!(conn, "SetChatRoomGroupAvatar", CChatRoomSetChatRoomGroupAvatarResponse, CChatRoomSetChatRoomGroupAvatarRequest { chat_group_id: g, avatar_sha: Some(sha) }),
                    Err(e) => Err(e),
                },
                Err(e) => Err(err(e)),
            },
            Some(Err(e)) => Err(e),
            None => Err("Not signed in on the web".into()),
        },
        AdminOp::CreateRoom { name, voice } => call!(conn, "CreateChatRoom", CChatRoomCreateChatRoomResponse, CChatRoomCreateChatRoomRequest { chat_group_id: g, name: Some(name.clone()), allow_voice: Some(*voice) }),
        AdminOp::DeleteRoom { room } => call!(conn, "DeleteChatRoom", CChatRoomDeleteChatRoomResponse, CChatRoomDeleteChatRoomRequest { chat_group_id: g, chat_id: Some(*room) }),
        AdminOp::RenameRoom { room, name } => call!(conn, "RenameChatRoom", CChatRoomRenameChatRoomResponse, CChatRoomRenameChatRoomRequest { chat_group_id: g, chat_id: Some(*room), name: Some(name.clone()) }),
        AdminOp::MoveRoom { room, after } => call!(conn, "ReorderChatRoom", CChatRoomReorderChatRoomResponse, CChatRoomReorderChatRoomRequest { chat_group_id: g, chat_id: Some(*room), move_after_chat_id: Some(*after) }),
        AdminOp::CreateRole { name } => call!(conn, "CreateRole", CChatRoomCreateRoleResponse, CChatRoomCreateRoleRequest { chat_group_id: g, name: Some(name.clone()) }),
        AdminOp::RenameRole { role, name } => call!(conn, "RenameRole", CChatRoomRenameRoleResponse, CChatRoomRenameRoleRequest { chat_group_id: g, role_id: Some(*role), name: Some(name.clone()) }),
        AdminOp::DeleteRole { role } => call!(conn, "DeleteRole", CChatRoomDeleteRoleResponse, CChatRoomDeleteRoleRequest { chat_group_id: g, role_id: Some(*role) }),
        AdminOp::SetRolePerms { role, perms } => call!(conn, "ReplaceRoleActions", CChatRoomReplaceRoleActionsResponse, CChatRoomReplaceRoleActionsRequest { chat_group_id: g, role_id: Some(*role), actions: Some(actions(*role, perms)) }),
        AdminOp::AddRole { steam_id, role } => call!(conn, "AddRoleToUser", CChatRoomAddRoleToUserResponse, CChatRoomAddRoleToUserRequest { chat_group_id: g, role_id: Some(*role), steamid: Some(*steam_id) }),
        AdminOp::RemoveRole { steam_id, role } => call!(conn, "DeleteRoleFromUser", CChatRoomDeleteRoleFromUserResponse, CChatRoomDeleteRoleFromUserRequest { chat_group_id: g, role_id: Some(*role), steamid: Some(*steam_id) }),
        AdminOp::MakeAdmin { steam_id, name } => make_admin(&conn, group, *steam_id, name).await,
        AdminOp::Kick { steam_id } => call!(conn, "KickUserFromGroup", CChatRoomKickUserResponse, CChatRoomKickUserRequest { chat_group_id: g, steamid: Some(*steam_id), expiration: Some(now()) }),
        AdminOp::Ban { steam_id, ban } => call!(conn, "SetUserBanState", CChatRoomSetUserBanStateResponse, CChatRoomSetUserBanStateRequest { chat_group_id: g, steamid: Some(*steam_id), ban_state: Some(*ban) }),
        AdminOp::Leave => call!(conn, "LeaveChatRoomGroup", CChatRoomLeaveChatRoomGroupResponse, CChatRoomLeaveChatRoomGroupRequest { chat_group_id: g }),
        AdminOp::Delete => delete(&conn, me, group).await,
        AdminOp::CreateGroup { name, invite } => {
            match create(&conn, name, invite).await {
                Ok(created) => {
                    sink(Event::ChatAdmin(AdminEvent::GroupCreated { group: created, members: vec![me] }));
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    };
    let ok = result.is_ok();
    sink(Event::ChatAdmin(AdminEvent::Done { group, op: op.clone(), result }));
    if !ok {
        return;
    }
    // Steam's fresh copy: the settings, and the group's name and channels.
    if op.reloads() {
        sink(Event::ChatAdmin(AdminEvent::Loaded { group, result: load(&conn, group).await }));
    }
    if matches!(op, AdminOp::Rename { .. } | AdminOp::SetTagline { .. } | AdminOp::SetAvatar { .. } | AdminOp::CreateRoom { .. } | AdminOp::DeleteRoom { .. } | AdminOp::RenameRoom { .. } | AdminOp::MoveRoom { .. } | AdminOp::Leave | AdminOp::Delete | AdminOp::Kick { .. } | AdminOp::Ban { .. })
    {
        if let Ok((groups, members)) = crate::groupchat::groups(&conn).await {
            sink(Event::ChatGroupMembers(members));
            sink(Event::ChatGroups(Ok(groups)));
        }
    }
}

/// A role that may do everything (made when there's none), given to someone.
async fn make_admin(conn: &Connection, group: u64, who: SteamId, name: &str) -> Result<(), String> {
    let a = load(conn, group).await?;
    let role = match a.roles.iter().find(|r| r.id != a.default_role && r.perms.is_all()) {
        Some(r) => r.id,
        None => {
            let r: CChatRoomCreateRoleResponse = conn.call("ChatRoom.CreateRole#1", &CChatRoomCreateRoleRequest { chat_group_id: Some(group), name: Some(name.to_string()) }).await.map_err(err)?;
            let id = r.actions.as_ref().map(|a| a.role_id()).filter(|id| *id != 0).ok_or("Steam didn't make the role")?;
            call!(conn, "ReplaceRoleActions", CChatRoomReplaceRoleActionsResponse, CChatRoomReplaceRoleActionsRequest { chat_group_id: Some(group), role_id: Some(id), actions: Some(actions(id, &RolePerms::ALL)) })?;
            id
        }
    };
    call!(conn, "AddRoleToUser", CChatRoomAddRoleToUserResponse, CChatRoomAddRoleToUserRequest { chat_group_id: Some(group), role_id: Some(role), steamid: Some(who) })
}

/// Steam has no "delete group chat": drop its invite links, remove
/// everyone else, then leave; the empty group goes away.
async fn delete(conn: &Connection, me: SteamId, group: u64) -> Result<(), String> {
    let a = load(conn, group).await?;
    if a.owner != me {
        return Err("Only the owner can delete a group chat".into());
    }
    for l in &a.links {
        let _ = call!(conn, "DeleteInviteLink", CChatRoomDeleteInviteLinkResponse, CChatRoomDeleteInviteLinkRequest { chat_group_id: Some(group), invite_code: Some(l.code.clone()) });
    }
    for i in &a.invited {
        let _ = call!(conn, "RevokeInviteToGroup", CChatRoomRevokeInviteResponse, CChatRoomRevokeInviteRequest { chat_group_id: Some(group), steamid: Some(i.steam_id) });
    }
    for m in a.members.iter().filter(|m| m.steam_id != me) {
        call!(conn, "KickUserFromGroup", CChatRoomKickUserResponse, CChatRoomKickUserRequest { chat_group_id: Some(group), steamid: Some(m.steam_id), expiration: Some(now()) })?;
    }
    call!(conn, "LeaveChatRoomGroup", CChatRoomLeaveChatRoomGroupResponse, CChatRoomLeaveChatRoomGroupRequest { chat_group_id: Some(group) })
}

/// A new group chat; with a name it's kept (Steam's "save") like Steam does
/// for named groups.
async fn create(conn: &Connection, name: &str, invite: &[SteamId]) -> Result<ChatGroup, String> {
    let req = CChatRoomCreateChatRoomGroupRequest { name: Some(name.to_string()), steamid_invitees: invite.to_vec(), ..Default::default() };
    let r: CChatRoomCreateChatRoomGroupResponse = conn.call("ChatRoom.CreateChatRoomGroup#1", &req).await.map_err(err)?;
    let id = r.chat_group_id();
    if !name.trim().is_empty() {
        let _ = call!(conn, "SaveChatRoomGroup", CChatRoomSaveChatRoomGroupResponse, CChatRoomSaveChatRoomGroupRequest { chat_group_id: Some(id), name: Some(name.to_string()) });
    }
    let mut g = group_from_state(id, &r.state.unwrap_or_default());
    if g.name.is_empty() {
        g.name = name.to_string();
    }
    Ok(g)
}

/// Upload a picture the way Steam's chat uploads images; returns its SHA-1,
/// which is what a group's icon is set to.
async fn upload_image(web: &Web, bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
    use sha1::{Digest, Sha1};
    let sha = Sha1::digest(bytes).to_vec();
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(err)?.into_dimensions().map_err(|_| "That file isn't a picture".to_string())?;
    let kind = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => "image/png",
        Ok(image::ImageFormat::Gif) => "image/gif",
        _ => "image/jpeg",
    };
    let fields = |extra: &[(&'static str, String)]| -> Vec<(&'static str, String)> {
        let mut f = vec![
            ("sessionid", web.session_id.clone()),
            ("l", "english".to_string()),
            ("file_size", bytes.len().to_string()),
            ("file_name", name.to_string()),
            ("file_sha", hex(&sha)),
            ("file_image_width", w.to_string()),
            ("file_image_height", h.to_string()),
            ("file_type", kind.to_string()),
        ];
        f.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        f
    };
    let begin: serde_json::Value = web.http.post(format!("{COMMUNITY}/chat/beginfileupload/")).header("Referer", format!("{COMMUNITY}/chat/")).form(&fields(&[])).send().await.map_err(err)?.json().await.map_err(err)?;
    if begin["success"].as_i64() != Some(1) {
        return Err(format!("Steam didn't take the picture ({})", begin["success"]));
    }
    let r = &begin["result"];
    let scheme = if r["use_https"].as_bool() == Some(true) || r["use_https"].as_i64() == Some(1) { "https" } else { "http" };
    let mut put = web.http.put(format!("{scheme}://{}{}", r["url_host"].as_str().unwrap_or_default(), r["url_path"].as_str().unwrap_or_default())).body(bytes.to_vec());
    for hd in r["request_headers"].as_array().into_iter().flatten() {
        let (k, v) = (hd["name"].as_str().unwrap_or_default(), hd["value"].as_str().unwrap_or_default());
        if !k.eq_ignore_ascii_case("content-length") && !k.eq_ignore_ascii_case("host") {
            put = put.header(k, v);
        }
    }
    let up = put.send().await.map_err(err)?;
    if !up.status().is_success() {
        return Err(format!("Upload failed ({})", up.status()));
    }
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let extra = [("success", "1".to_string()), ("ugcid", s(&r["ugcid"])), ("timestamp", s(&r["timestamp"])), ("hmac", s(&r["hmac"])), ("spoiler", "0".to_string())];
    let commit: serde_json::Value = web.http.post(format!("{COMMUNITY}/chat/commitfileupload/")).header("Referer", format!("{COMMUNITY}/chat/")).form(&fields(&extra)).send().await.map_err(err)?.json().await.map_err(err)?;
    if commit["success"].as_i64() != Some(1) {
        return Err(format!("Steam didn't keep the picture ({})", commit["success"]));
    }
    Ok(sha)
}

/// What a group chat change does to the app's state.
pub fn apply(state: &mut crate::State, e: &AdminEvent) {
    let me = state.steam_id;
    match e {
        AdminEvent::Loaded { group, result } => {
            // A failed refresh keeps what was there.
            if result.is_ok() || !matches!(state.chat_admin.get(group), Some(Ok(_))) {
                state.chat_admin.insert(*group, result.clone());
            }
        }
        AdminEvent::LinkCreated { group, link } => {
            if let Some(Ok(a)) = state.chat_admin.get_mut(group) {
                let mut link = link.clone();
                if link.creator == 0 {
                    link.creator = me;
                }
                a.links.retain(|l| l.code != link.code);
                a.links.push(link);
            }
        }
        AdminEvent::GroupCreated { group, members } => {
            let mut members = members.clone();
            members.retain(|m| *m != 0);
            if members.is_empty() {
                members.push(me);
            }
            state.chat_group_members.insert(group.id, members.clone());
            state.chat_admin.insert(
                group.id,
                Ok(GroupAdmin {
                    group: group.id,
                    owner: me,
                    default_role: 1,
                    roles: vec![Role { id: 1, name: "Everyone".into(), ordinal: 0, perms: RolePerms { chat: true, mention_all: true, broadcasts: true, invite: true, change_details: true, channels: true, view_history: true, ..Default::default() } }],
                    members: members.iter().map(|m| Member { steam_id: *m, roles: vec![] }).collect(),
                    notify: Notify::All,
                    dms_allowed: true,
                    ..Default::default()
                }),
            );
            match &mut state.chat_groups {
                Some(Ok(list)) => {
                    if !list.iter().any(|g| g.id == group.id) {
                        list.insert(0, group.clone());
                    }
                }
                _ => state.chat_groups = Some(Ok(vec![group.clone()])),
            }
        }
        AdminEvent::Done { group, op, result: Ok(()) } => {
            if let Some(Ok(a)) = state.chat_admin.get_mut(group) {
                a.apply(op);
            }
            let groups = match &mut state.chat_groups {
                Some(Ok(list)) => list,
                _ => return,
            };
            match op {
                AdminOp::Leave | AdminOp::Delete => {
                    groups.retain(|g| g.id != *group);
                    state.chat_admin.remove(group);
                    state.chat_group_members.remove(group);
                }
                AdminOp::Kick { steam_id } | AdminOp::Ban { steam_id, ban: true } => {
                    if let Some(m) = state.chat_group_members.get_mut(group) {
                        m.retain(|x| x != steam_id);
                    }
                }
                _ => {
                    let Some(g) = groups.iter_mut().find(|g| g.id == *group) else { return };
                    edit_group(g, op);
                }
            }
        }
        AdminEvent::Done { .. } => {}
    }
}

/// A finished change to a group's name, tagline or channels, shown before
/// Steam's fresh list arrives (and in the demo).
fn edit_group(g: &mut ChatGroup, op: &AdminOp) {
    match op {
        AdminOp::Rename { name } => g.name = name.clone(),
        AdminOp::SetTagline { tagline } => g.tagline = tagline.clone(),
        AdminOp::CreateRoom { name, voice } => {
            // Made-up ids only until Steam's list replaces them.
            let id = g.rooms.iter().map(|r| r.id).max().unwrap_or(0) + 1;
            let sort = g.rooms.iter().map(|r| r.sort).max().unwrap_or(0) + 1;
            g.rooms.push(ChatRoom { id, name: name.clone(), voice: *voice, sort, ..Default::default() });
        }
        AdminOp::DeleteRoom { room } => g.rooms.retain(|r| r.id != *room),
        AdminOp::RenameRoom { room, name } => {
            if let Some(r) = g.rooms.iter_mut().find(|r| r.id == *room) {
                r.name = name.clone();
            }
        }
        AdminOp::MoveRoom { room, after } => {
            if let Some(i) = g.rooms.iter().position(|r| r.id == *room) {
                let r = g.rooms.remove(i);
                let at = if *after == 0 { 0 } else { g.rooms.iter().position(|x| x.id == *after).map(|p| p + 1).unwrap_or(g.rooms.len()) };
                g.rooms.insert(at, r);
                for (n, r) in g.rooms.iter_mut().enumerate() {
                    r.sort = n as u32;
                }
            }
        }
        _ => {}
    }
}

/// The demo (signed out): changes just happen, with made-up links and ids.
pub fn offline(group: u64, op: AdminOp, me: SteamId, sink: &Sink) {
    match &op {
        AdminOp::Load => return,
        AdminOp::CreateInviteLink { seconds, room } => {
            let code: String = (0..10).map(|_| char::from(b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789"[rand::random::<u8>() as usize % 56])).collect();
            let expires = seconds.map(|s| now() as u32 + s).unwrap_or(0);
            sink(Event::ChatAdmin(AdminEvent::LinkCreated { group, link: InviteLink { code, creator: me, expires, room: *room } }));
        }
        AdminOp::CreateGroup { name, invite } => {
            let id = 4_200_100 + rand::random::<u16>() as u64;
            let g = ChatGroup { id, name: name.clone(), default_room: id * 10, active_members: 1, rooms: vec![ChatRoom { id: id * 10, name: "Home".into(), ..Default::default() }], ..Default::default() };
            let _ = invite;
            sink(Event::ChatAdmin(AdminEvent::GroupCreated { group: g, members: vec![me] }));
        }
        _ => {}
    }
    sink(Event::ChatAdmin(AdminEvent::Done { group, op, result: Ok(()) }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_toggles_round_trip() {
        let p = RolePerms { chat: true, kick: true, channels: true, assign_roles: true, ..Default::default() };
        let a = actions(9, &p);
        assert_eq!(a.role_id(), 9);
        assert!(a.can_kick() && a.can_chat() && a.can_create_rename_delete_channel() && a.can_change_user_roles());
        assert!(!a.can_ban() && !a.can_change_group_roles());
        assert_eq!(perms(&a), p);
    }

    #[test]
    fn reads_owner_roles_members_and_settings() {
        let st = CChatRoomGroupState {
            header_state: Some(CChatRoomGroupHeaderState {
                chat_group_id: Some(5),
                accountid_owner: Some(1),
                default_role_id: Some(100),
                roles: vec![CChatRole { role_id: Some(100), name: Some("Everyone".into()), ordinal: Some(0) }, CChatRole { role_id: Some(101), name: Some("Mods".into()), ordinal: Some(1) }],
                role_actions: vec![CChatRoleActions { role_id: Some(100), can_chat: Some(true), ..Default::default() }, CChatRoleActions { role_id: Some(101), can_kick: Some(true), can_ban: Some(true), ..Default::default() }],
                ..Default::default()
            }),
            members: vec![
                CChatRoomMember { accountid: Some(1), state: Some(EChatRoomJoinState::KEChatRoomJoinStateJoined as i32), role_ids: vec![100], ..Default::default() },
                CChatRoomMember { accountid: Some(2), state: Some(EChatRoomJoinState::KEChatRoomJoinStateJoined as i32), role_ids: vec![100, 101], ..Default::default() },
                CChatRoomMember { accountid: Some(3), state: Some(EChatRoomJoinState::KEChatRoomJoinStateNone as i32), ..Default::default() },
            ],
            ..Default::default()
        };
        let user = CUserChatRoomGroupState {
            chat_group_id: Some(5),
            desktop_notification_level: Some(EChatRoomNotificationLevel::KEChatroomNotificationLevelMentionMe as i32),
            direct_messages_allowed: Some(true),
            user_chat_room_state: vec![
                CUserChatRoomState { chat_id: Some(50), desktop_notification_level: Some(EChatRoomNotificationLevel::KEChatroomNotificationLevelNone as i32), ..Default::default() },
                CUserChatRoomState { chat_id: Some(51), ..Default::default() },
            ],
            ..Default::default()
        };
        let a = parse(5, &st, Some(&user));
        assert_eq!(a.owner, INDIVIDUAL + 1);
        assert_eq!(a.members.len(), 2, "only people who joined");
        assert_eq!(a.members[1].roles, vec![101], "the default role isn't listed");
        let mod_perms = a.perms_of(INDIVIDUAL + 2);
        assert!(mod_perms.kick && mod_perms.ban && mod_perms.chat);
        assert_eq!(a.notify, Notify::Mentions);
        assert_eq!(a.room_notify(50), Notify::None);
        assert_eq!(a.room_notify(51), Notify::Inherit);
        assert!(a.dms_allowed);
        // Never picked: Steam's "every message".
        assert_eq!(parse(5, &st, None).notify, Notify::All);
    }

    #[test]
    fn builds_notification_requests() {
        let r = prefs_request(5, &AdminOp::SetNotify(Notify::MentionAll)).unwrap();
        assert_eq!(r.chat_group_id(), 5);
        assert_eq!(r.chat_group_preferences.as_ref().unwrap().desktop_notification_level(), EChatRoomNotificationLevel::KEChatroomNotificationLevelMentionAll);
        assert!(r.chat_room_preferences.is_empty());
        let r = prefs_request(5, &AdminOp::SetRoomNotify { room: 50, level: Notify::Inherit }).unwrap();
        let room = &r.chat_room_preferences[0];
        assert_eq!((room.chat_id(), room.desktop_notification_level()), (50, EChatRoomNotificationLevel::KEChatroomNotificationLevelInvalid), "don't override");
        let r = prefs_request(5, &AdminOp::SetDmsAllowed(false)).unwrap();
        assert_eq!(r.chat_group_preferences.unwrap().direct_messages_allowed, Some(false));
        assert!(prefs_request(5, &AdminOp::Leave).is_none());
    }

    #[test]
    fn builds_invite_link_requests() {
        let r = link_request(5, Some(3600), 0);
        assert_eq!((r.chat_group_id(), r.seconds_valid(), r.chat_id), (5, 3600, None));
        assert_eq!(link_request(5, None, 50).seconds_valid(), 0, "never expires");
        assert_eq!(InviteLink { code: "Z0uG8hG0".into(), ..Default::default() }.url(), "https://s.team/chat/Z0uG8hG0");
    }

    /// The demo's way: changes come back as events and show in the state.
    #[test]
    fn signed_out_changes_show_in_state() {
        use std::sync::Mutex;
        let me = INDIVIDUAL + 1;
        let mut state = crate::State { steam_id: me, ..Default::default() };
        let got: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink: Sink = {
            let got = got.clone();
            Arc::new(move |e| got.lock().unwrap().push(e))
        };
        let run = |state: &mut crate::State, group: u64, op: AdminOp| {
            offline(group, op, 0, &sink);
            for e in got.lock().unwrap().drain(..) {
                state.apply(&e);
            }
        };
        run(&mut state, 0, AdminOp::CreateGroup { name: "Pals".into(), invite: vec![] });
        let g = state.chat_groups.as_ref().unwrap().as_ref().unwrap()[0].clone();
        assert_eq!(g.name, "Pals");
        assert_eq!(state.chat_group_members[&g.id], vec![me]);
        let admin = |s: &crate::State| s.chat_admin[&g.id].clone().unwrap();
        assert_eq!(admin(&state).owner, me, "you own a group you made");

        run(&mut state, g.id, AdminOp::CreateInviteLink { seconds: None, room: 0 });
        let links = admin(&state).links;
        assert_eq!((links.len(), links[0].expires, links[0].creator), (1, 0, me));
        run(&mut state, g.id, AdminOp::DeleteInviteLink { code: links[0].code.clone() });
        assert!(admin(&state).links.is_empty());

        run(&mut state, g.id, AdminOp::CreateRoom { name: "memes".into(), voice: false });
        run(&mut state, g.id, AdminOp::Rename { name: "Buddies".into() });
        let g2 = state.chat_groups.as_ref().unwrap().as_ref().unwrap()[0].clone();
        assert_eq!(g2.name, "Buddies");
        assert_eq!(g2.rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Home", "memes"]);
        let memes = g2.rooms[1].id;
        run(&mut state, g.id, AdminOp::MoveRoom { room: memes, after: 0 });
        assert_eq!(state.chat_groups.as_ref().unwrap().as_ref().unwrap()[0].rooms[0].id, memes);

        run(&mut state, g.id, AdminOp::Delete);
        assert!(state.chat_groups.as_ref().unwrap().as_ref().unwrap().is_empty());
        assert!(!state.chat_admin.contains_key(&g.id));
    }

    #[test]
    fn new_group_from_state() {
        let st = CChatRoomGroupState {
            header_state: Some(CChatRoomGroupHeaderState { chat_name: Some("Pals".into()), ..Default::default() }),
            default_chat_id: Some(70),
            chat_rooms: vec![CChatRoomState { chat_id: Some(70), chat_name: Some("Home".into()), ..Default::default() }],
            ..Default::default()
        };
        let g = group_from_state(7, &st);
        assert_eq!((g.id, g.name.as_str(), g.default_room, g.rooms.len()), (7, "Pals", 70, 1));
    }
}

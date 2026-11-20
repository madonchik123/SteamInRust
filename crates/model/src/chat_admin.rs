//! Managing a Steam group chat: its roles and what each may do, members'
//! roles, invite links, invited accounts, bans and your notification
//! settings for it (Steam's group chat settings and notifications dialogs).

use crate::SteamId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Steam ids of individual accounts are this plus the account id.
pub const INDIVIDUAL: u64 = 76561197960265728;

/// When to be notified about a group chat or one of its text channels
/// (Steam's "Notify me for"). `Inherit` is a channel's "Don't override".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Notify {
    #[default]
    Inherit,
    None,
    /// Direct @mentions only.
    Mentions,
    /// @all, @online and @mentions.
    MentionAll,
    /// Every message.
    All,
}

impl Notify {
    /// Steam's `EChatRoomNotificationLevel` (0 = not set).
    pub fn level(self) -> i32 {
        match self {
            Notify::Inherit => 0,
            Notify::None => 1,
            Notify::Mentions => 2,
            Notify::MentionAll => 3,
            Notify::All => 4,
        }
    }
    pub fn from_level(n: i32) -> Notify {
        match n {
            1 => Notify::None,
            2 => Notify::Mentions,
            3 => Notify::MentionAll,
            4 => Notify::All,
            _ => Notify::Inherit,
        }
    }
}

/// What a role allows (Steam's role toggles).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolePerms {
    pub chat: bool,
    pub mention_all: bool,
    pub broadcasts: bool,
    pub kick: bool,
    pub ban: bool,
    pub invite: bool,
    pub edit_roles: bool,
    pub assign_roles: bool,
    pub change_details: bool,
    pub channels: bool,
    pub view_history: bool,
}

impl RolePerms {
    pub const ALL: RolePerms = RolePerms {
        chat: true,
        mention_all: true,
        broadcasts: true,
        kick: true,
        ban: true,
        invite: true,
        edit_roles: true,
        assign_roles: true,
        change_details: true,
        channels: true,
        view_history: true,
    };
    pub fn or(self, o: RolePerms) -> RolePerms {
        RolePerms {
            chat: self.chat || o.chat,
            mention_all: self.mention_all || o.mention_all,
            broadcasts: self.broadcasts || o.broadcasts,
            kick: self.kick || o.kick,
            ban: self.ban || o.ban,
            invite: self.invite || o.invite,
            edit_roles: self.edit_roles || o.edit_roles,
            assign_roles: self.assign_roles || o.assign_roles,
            change_details: self.change_details || o.change_details,
            channels: self.channels || o.channels,
            view_history: self.view_history || o.view_history,
        }
    }
    /// Everything a role can be given (an "admin").
    pub fn is_all(&self) -> bool {
        *self == RolePerms::ALL
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Role {
    pub id: u64,
    pub name: String,
    pub ordinal: u32,
    pub perms: RolePerms,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub steam_id: SteamId,
    /// Role ids beyond the default one.
    pub roles: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InviteLink {
    pub code: String,
    pub creator: SteamId,
    /// Unix seconds; 0 = never.
    pub expires: u32,
    pub room: u64,
}

impl InviteLink {
    pub fn url(&self) -> String {
        format!("https://s.team/chat/{}", self.code)
    }
}

/// Someone invited who hasn't joined yet, or someone banned.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub steam_id: SteamId,
    /// Who invited or banned them.
    pub by: SteamId,
    pub time: u32,
    pub reason: String,
}

/// Everything the settings and notifications dialogs show for a group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupAdmin {
    pub group: u64,
    pub owner: SteamId,
    /// The "Everyone" role.
    pub default_role: u64,
    pub roles: Vec<Role>,
    pub members: Vec<Member>,
    /// Your "Notify me for" and each channel's override.
    pub notify: Notify,
    pub room_notify: Vec<(u64, Notify)>,
    /// Members may message you directly.
    pub dms_allowed: bool,
    pub links: Vec<InviteLink>,
    pub invited: Vec<Listed>,
    pub bans: Vec<Listed>,
}

impl GroupAdmin {
    pub fn role(&self, id: u64) -> Option<&Role> {
        self.roles.iter().find(|r| r.id == id)
    }
    pub fn roles_of(&self, who: SteamId) -> Vec<u64> {
        self.members.iter().find(|m| m.steam_id == who).map(|m| m.roles.clone()).unwrap_or_default()
    }
    /// What `who` may do: the owner everything, others what the default
    /// role and their own roles allow.
    pub fn perms_of(&self, who: SteamId) -> RolePerms {
        if who == self.owner {
            return RolePerms::ALL;
        }
        let mut p = self.role(self.default_role).map(|r| r.perms).unwrap_or_default();
        for id in self.roles_of(who) {
            if let Some(r) = self.role(id) {
                p = p.or(r.perms);
            }
        }
        p
    }
    /// Roles in Steam's order (by ordinal), the default one last like Steam.
    pub fn sorted_roles(&self) -> Vec<&Role> {
        let mut v: Vec<&Role> = self.roles.iter().collect();
        v.sort_by_key(|r| (r.id == self.default_role, r.ordinal, r.id));
        v
    }
    pub fn room_notify(&self, room: u64) -> Notify {
        self.room_notify.iter().find(|(r, _)| *r == room).map(|(_, n)| *n).unwrap_or_default()
    }

    /// Shows what a finished change did, before Steam's fresh copy arrives
    /// (and in the demo, where none does).
    pub fn apply(&mut self, op: &AdminOp) {
        match op {
            AdminOp::SetNotify(n) => self.notify = *n,
            AdminOp::SetRoomNotify { room, level } => {
                self.room_notify.retain(|(r, _)| r != room);
                if *level != Notify::Inherit {
                    self.room_notify.push((*room, *level));
                }
            }
            AdminOp::SetDmsAllowed(on) => self.dms_allowed = *on,
            AdminOp::DeleteInviteLink { code } => self.links.retain(|l| &l.code != code),
            AdminOp::RenameRole { role, name } => {
                if let Some(r) = self.roles.iter_mut().find(|r| r.id == *role) {
                    r.name = name.clone();
                }
            }
            AdminOp::DeleteRole { role } => {
                self.roles.retain(|r| r.id != *role);
                for m in &mut self.members {
                    m.roles.retain(|r| r != role);
                }
            }
            AdminOp::SetRolePerms { role, perms } => {
                if let Some(r) = self.roles.iter_mut().find(|r| r.id == *role) {
                    r.perms = *perms;
                }
            }
            AdminOp::AddRole { steam_id, role } => match self.members.iter_mut().find(|m| m.steam_id == *steam_id) {
                Some(m) if !m.roles.contains(role) => m.roles.push(*role),
                Some(_) => {}
                None => self.members.push(Member { steam_id: *steam_id, roles: vec![*role] }),
            },
            AdminOp::RemoveRole { steam_id, role } => {
                if let Some(m) = self.members.iter_mut().find(|m| m.steam_id == *steam_id) {
                    m.roles.retain(|r| r != role);
                }
            }
            AdminOp::Kick { steam_id } => self.members.retain(|m| m.steam_id != *steam_id),
            AdminOp::Ban { steam_id, ban } => {
                self.bans.retain(|b| b.steam_id != *steam_id);
                if *ban {
                    self.members.retain(|m| m.steam_id != *steam_id);
                    self.bans.push(Listed { steam_id: *steam_id, ..Default::default() });
                }
            }
            AdminOp::RevokeInvite { steam_id } => self.invited.retain(|i| i.steam_id != *steam_id),
            AdminOp::InviteFriend { steam_id, .. } => {
                if !self.invited.iter().any(|i| i.steam_id == *steam_id) {
                    self.invited.push(Listed { steam_id: *steam_id, ..Default::default() });
                }
            }
            _ => {}
        }
    }
}

/// A change to a group chat (or reading its settings). Sent as
/// `Command::ChatAdmin { group, op }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AdminOp {
    /// Read roles, members' roles, links, invites, bans and your settings.
    Load,
    SetNotify(Notify),
    /// A channel's override (`Inherit` = don't override).
    SetRoomNotify { room: u64, level: Notify },
    SetDmsAllowed(bool),
    /// `seconds` `None` = never expires.
    CreateInviteLink { seconds: Option<u32>, room: u64 },
    DeleteInviteLink { code: String },
    InviteFriend { steam_id: SteamId, room: u64 },
    RevokeInvite { steam_id: SteamId },
    Rename { name: String },
    SetTagline { tagline: String },
    /// Upload a picture as the group's icon.
    SetAvatar { path: PathBuf },
    CreateRoom { name: String, voice: bool },
    DeleteRoom { room: u64 },
    RenameRoom { room: u64, name: String },
    /// Put a channel after another (0 = first).
    MoveRoom { room: u64, after: u64 },
    CreateRole { name: String },
    RenameRole { role: u64, name: String },
    DeleteRole { role: u64 },
    SetRolePerms { role: u64, perms: RolePerms },
    AddRole { steam_id: SteamId, role: u64 },
    RemoveRole { steam_id: SteamId, role: u64 },
    /// Give someone a role that may do everything (made, as "Admin", when
    /// the group has none).
    MakeAdmin { steam_id: SteamId, name: String },
    Kick { steam_id: SteamId },
    Ban { steam_id: SteamId, ban: bool },
    Leave,
    /// There's no "delete" in Steam: remove everyone else, drop the invite
    /// links, then leave, and Steam drops the empty group.
    Delete,
    /// A new group chat (`group` 0) with friends invited to it.
    CreateGroup { name: String, invite: Vec<SteamId> },
}

impl AdminOp {
    /// Changes Steam's copy of the group's settings (it's read again after).
    pub fn reloads(&self) -> bool {
        !matches!(self, AdminOp::Load | AdminOp::Leave | AdminOp::Delete | AdminOp::CreateGroup { .. })
    }
}

/// What a [`AdminOp`] came back with (`Event::ChatAdmin`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AdminEvent {
    Loaded { group: u64, result: Result<GroupAdmin, String> },
    Done { group: u64, op: AdminOp, result: Result<(), String> },
    /// A new invite link (also in the group's list).
    LinkCreated { group: u64, link: InviteLink },
    /// A group chat was made; it opens.
    GroupCreated { group: crate::ChatGroup, members: Vec<SteamId> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> GroupAdmin {
        GroupAdmin {
            group: 1,
            owner: INDIVIDUAL + 1,
            default_role: 10,
            roles: vec![
                Role { id: 10, name: "Everyone".into(), ordinal: 0, perms: RolePerms { chat: true, invite: true, ..Default::default() } },
                Role { id: 11, name: "Mods".into(), ordinal: 1, perms: RolePerms { kick: true, ..Default::default() } },
            ],
            members: vec![Member { steam_id: INDIVIDUAL + 2, roles: vec![11] }, Member { steam_id: INDIVIDUAL + 3, roles: vec![] }],
            ..Default::default()
        }
    }

    #[test]
    fn permissions_add_up() {
        let a = admin();
        assert!(a.perms_of(INDIVIDUAL + 1).is_all(), "the owner may do everything");
        let m = a.perms_of(INDIVIDUAL + 2);
        assert!(m.kick && m.chat && m.invite && !m.ban);
        let e = a.perms_of(INDIVIDUAL + 3);
        assert!(e.chat && !e.kick);
        assert_eq!(a.sorted_roles().last().map(|r| r.id), Some(10), "Everyone last");
    }

    #[test]
    fn changes_show_locally() {
        let mut a = admin();
        a.apply(&AdminOp::AddRole { steam_id: INDIVIDUAL + 3, role: 11 });
        assert!(a.perms_of(INDIVIDUAL + 3).kick);
        a.apply(&AdminOp::DeleteRole { role: 11 });
        assert!(!a.perms_of(INDIVIDUAL + 2).kick);
        a.apply(&AdminOp::Ban { steam_id: INDIVIDUAL + 2, ban: true });
        assert_eq!(a.bans.len(), 1);
        assert!(a.members.iter().all(|m| m.steam_id != INDIVIDUAL + 2));
        a.apply(&AdminOp::Ban { steam_id: INDIVIDUAL + 2, ban: false });
        assert!(a.bans.is_empty());
        a.apply(&AdminOp::SetRoomNotify { room: 5, level: Notify::None });
        assert_eq!(a.room_notify(5), Notify::None);
        a.apply(&AdminOp::SetRoomNotify { room: 5, level: Notify::Inherit });
        assert_eq!(a.room_notify(5), Notify::Inherit);
    }

    #[test]
    fn notify_levels_round_trip() {
        for n in [Notify::Inherit, Notify::None, Notify::Mentions, Notify::MentionAll, Notify::All] {
            assert_eq!(Notify::from_level(n.level()), n);
        }
    }
}

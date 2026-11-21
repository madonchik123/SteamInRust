//! Managing a Steam Family in Settings: invite and cancel, accept and
//! decline, remove and leave, through Steam's FamilyGroups service.
//!
//! Every action goes through a dialog that says what it will do first.
//! Those that can lock someone out of a family for up to a year (accepting,
//! removing, leaving) also need an "I understand" tick. In `--demo` nothing
//! is ever sent: the dialogs walk through made-up answers.
//!
//! Stays on Steam's site, because the FamilyGroups service doesn't cover it
//! or Steam only offers it there: creating, renaming or deleting a family,
//! its history, and a child's limits (Steam's separate Parental service).

use super::settings::web;
use client::Command;
use egui::{Align, Align2, Layout, Rect, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{Family, FamilyAction, FamilyInvite, FamilyMember, FamilyOutcome, FamilyRole, FamilyStatus, InviteBlock, SteamId, art};
use ui::{Theme, icons, widgets as w};

/// Steam's own family page, for what only it can do.
const FAMILY_PAGE: &str = "https://store.steampowered.com/account/familymanagement";

/// The open dialog.
enum Dialog {
    Invite { family: u64, who: Option<SteamId>, role: FamilyRole },
    CancelInvite { family: u64, steam_id: SteamId },
    Accept(FamilyInvite),
    Decline(FamilyInvite),
    Remove { family: u64, member: FamilyMember },
    Leave { family: u64, cooldown: u32 },
    /// Steam wants one more confirmation (phone or email).
    Waiting { action: FamilyAction, outcome: FamilyOutcome, invite: Option<FamilyInvite> },
}

#[derive(Default)]
pub struct FamilyManager {
    dialog: Option<Dialog>,
    /// The "I understand" tick in the open dialog.
    understood: bool,
    /// Waiting for Steam's answer.
    busy: bool,
    error: Option<String>,
    search: String,
    code: String,
}

fn language() -> String {
    i18n::language().steam_name().to_string()
}

fn days(seconds: u32) -> u32 {
    seconds.div_ceil(86_400)
}

/// The date `seconds` from now.
fn date_in(seconds: u32) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    ui::date(now + u64::from(seconds))
}

/// The code from Steam's email (a number).
fn parse_code(s: &str) -> Option<u64> {
    let s = s.trim();
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// What the demo answers instead of Steam.
fn demo_outcome(action: &FamilyAction) -> FamilyOutcome {
    match action {
        FamilyAction::Invite { .. } => FamilyOutcome::ConfirmOnPhone,
        FamilyAction::Accept { .. } => FamilyOutcome::ConfirmByEmail,
        _ => FamilyOutcome::Done,
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).wrap());
}

/// One rule, with a dot before it.
fn rule(ui: &mut Ui, t: &Theme, text: &str, color: egui::Color32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        ui.label(w::body_m(t, "•", color));
        wrapped(ui, w::body_m(t, text, color));
    });
}

fn role_name(role: FamilyRole) -> &'static str {
    match role {
        FamilyRole::Adult => t!("Adult"),
        FamilyRole::Child => t!("Child"),
        FamilyRole::Unknown => t!("Member"),
    }
}

/// What a child's role means, shown wherever one is chosen or accepted.
fn child_warning(ui: &mut Ui, t: &Theme) {
    rule(ui, t, t!("Any adult in the family gets full control over a child's account: playtime, games, store and community."), t.danger);
    rule(ui, t, t!("A child can't leave a family; only an adult in it or Steam Support can remove them."), t.danger);
    rule(ui, t, t!("Only join as a child if a parent or guardian you trust invited you. Steam Support will never ask you to."), t.danger);
}

/// Someone to choose in the invite dialog. True when clicked.
fn person_row(ui: &mut Ui, t: &Theme, avatar: &str, name: &str, note: &str, on: bool) -> bool {
    let c = &t.components.menu_item;
    let h = t.dense(44.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if ui.is_rect_visible(r) {
        let look = t.look(c, w::interact(ui, &resp, on));
        w::paint_box(ui, t, r, c, &look, 0.0);
        let pad = t.spacing.sm;
        let size = h - pad * 2.0;
        let pic = Rect::from_min_size(r.min + vec2(pad, pad), vec2(size, size));
        if !w::paint_cover(ui, pic, avatar, t.corner_small()) {
            w::skeleton(ui, t, pic, t.corner_small());
        }
        let right = if on {
            ui.painter().text(egui::pos2(r.right() - pad, r.center().y), Align2::RIGHT_CENTER, icons::CHECK, t.typography.body_l.font(), t.accent);
            r.right() - pad * 2.0 - t.typography.body_l.size
        } else {
            r.right() - pad
        };
        let x = pic.right() + pad;
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), t.typography.body_m.font(), t.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
        let g = ui.painter().layout_job(job);
        let y = if note.is_empty() { r.center().y - g.size().y / 2.0 } else { r.center().y - g.size().y };
        ui.painter().galley(egui::pos2(x, y), g, t.text);
        if !note.is_empty() {
            ui.painter().text(egui::pos2(x, r.center().y), Align2::LEFT_TOP, note, t.typography.body_s.font(), t.text_dim);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

impl FamilyManager {
    fn open(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.understood = false;
        self.busy = false;
        self.error = None;
        self.search.clear();
        self.code.clear();
    }

    fn close(&mut self) {
        self.dialog = None;
        self.busy = false;
        self.error = None;
    }

    /// Send a confirmed action to Steam (never in the demo).
    fn send(&mut self, ctx: &mut Ctx, action: FamilyAction) {
        self.error = None;
        if crate::app::demo() {
            let outcome = demo_outcome(&action);
            self.answered(ctx, action, Ok(outcome));
            return;
        }
        self.busy = true;
        ctx.send(Command::FamilyAction { action, language: language() });
    }

    /// Steam answered (see [`client::Event::FamilyActionDone`]).
    pub fn event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        if let client::Event::FamilyActionDone { action, result } = event {
            self.answered(ctx, action.clone(), result.clone());
        }
    }

    fn answered(&mut self, ctx: &mut Ctx, action: FamilyAction, result: Result<FamilyOutcome, String>) {
        self.busy = false;
        match result {
            Err(e) => {
                let e = t!("Steam didn't do it: {error}", error = i18n::tr_dyn(&e));
                if self.dialog.is_some() {
                    self.error = Some(e);
                } else {
                    ctx.toast(e);
                }
            }
            Ok(FamilyOutcome::Done) => {
                self.close();
                ctx.toast(match action {
                    FamilyAction::Invite { .. } => t!("Invitation sent"),
                    FamilyAction::CancelInvite { .. } => t!("Invitation canceled"),
                    FamilyAction::Accept { .. } | FamilyAction::ConfirmJoin { .. } => t!("You joined the family"),
                    FamilyAction::Decline { .. } => t!("Invitation declined"),
                    FamilyAction::Remove { .. } => t!("Removed from the family"),
                    FamilyAction::Leave { .. } => t!("You left the family"),
                });
            }
            Ok(outcome) => {
                let invite = match self.dialog.take() {
                    Some(Dialog::Accept(i)) => Some(i),
                    Some(Dialog::Waiting { invite, .. }) => invite,
                    _ => None,
                };
                self.open(Dialog::Waiting { action, outcome, invite });
            }
        }
    }

    // ---------- on the page ----------

    /// Steam's rules, above everything you can do.
    pub fn rules(&self, ui: &mut Ui, t: &Theme) {
        w::panel(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            ui.label(w::heading_s(t, t!("Rules and cooldowns"), t.text));
            rule(ui, t, t!("A family is for one household, up to six accounts."), t.text_dim);
            rule(ui, t, t!("After joining a family you can't join a different one for a year, even if you leave or are removed."), t.text_dim);
            rule(ui, t, t!("When someone leaves, their place can stay closed to new members for up to a year. Former members can still be invited back."), t.text_dim);
            rule(ui, t, t!("Adults invite, remove and set limits. Children can't leave on their own."), t.text_dim);
        });
    }

    /// Invitations to you: who, as what, Accept and Decline.
    pub fn invites_to_you(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, status: &FamilyStatus) {
        for i in &status.invites {
            let p = ctx.state.persona(i.inviter);
            ui.add_space(t.spacing.sm);
            w::panel(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), t.dense(40.0), None);
                    let right = ui.available_width() - t.dense(230.0);
                    ui.allocate_ui_with_layout(vec2(right.max(t.dense(120.0)), 0.0), Layout::top_down(Align::Min), |ui| {
                        wrapped(ui, w::body_m(t, t!("{name} invited you to their family.", name = p.display_name()), t.text));
                        let mut line = t!("You'd join as: {role}", role = role_name(i.role)).to_string();
                        if i.awaiting_confirmation {
                            line.push_str(&format!("  ·  {}", t!("waiting for your confirmation")));
                        }
                        wrapped(ui, w::body_s(t, line, t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::button(ui, t, None, t!("Decline"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.open(Dialog::Decline(i.clone()));
                        }
                        if w::button(ui, t, Some(icons::CHECK), t!("Accept"), w::Kind::Primary, w::Size::Sm).clicked() {
                            self.open(Dialog::Accept(i.clone()));
                        }
                    });
                });
            });
        }
    }

    /// The Invite button, or why you can't.
    pub fn invite_button(&mut self, ui: &mut Ui, t: &Theme, status: &FamilyStatus, family: &Family) {
        let block = status.invite_block();
        // During a slot cooldown you can still invite former members back.
        let usable = block.is_none() || (matches!(block, Some(InviteBlock::SlotCooldown(_))) && !family.former.is_empty());
        if block == Some(InviteBlock::NotAdult) {
            return;
        }
        let tip = match block {
            Some(InviteBlock::Full) => t!("All six places are taken.").to_string(),
            Some(InviteBlock::SlotCooldown(s)) => t!("You can't invite new members for {days} more days. Former members can still be invited back.", days = days(s)),
            _ => t!("Invite someone to the family").to_string(),
        };
        let r = ui.add_enabled_ui(usable, |ui| w::button(ui, t, Some(icons::USER_PLUS), t!("Invite"), w::Kind::Primary, w::Size::Sm)).inner;
        if r.on_hover_text(&tip).on_disabled_hover_text(&tip).clicked() {
            self.open(Dialog::Invite { family: family.id, who: None, role: FamilyRole::Adult });
        }
    }

    /// Buttons on a member's row (drawn right to left).
    pub fn member_actions(&mut self, ui: &mut Ui, t: &Theme, status: &FamilyStatus, family: &Family, m: &FamilyMember, me: SteamId) {
        if status.can_remove(m.steam_id) {
            if m.steam_id == me {
                if w::button(ui, t, Some(icons::SIGN_OUT), t!("Leave family"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    self.open(Dialog::Leave { family: family.id, cooldown: m.cooldown });
                }
            } else if w::button(ui, t, None, t!("Remove"), w::Kind::Ghost, w::Size::Sm).clicked() {
                self.open(Dialog::Remove { family: family.id, member: m.clone() });
            }
        }
        // A child's limits are Steam's Parental service: on its site.
        if status.role == FamilyRole::Adult && m.role == FamilyRole::Child {
            let r = w::button(ui, t, Some(icons::EXTERNAL), t!("Limits"), w::Kind::Ghost, w::Size::Sm).on_hover_text(t!("Playtime, games, store and community limits open on Steam's site"));
            if r.clicked() {
                web(&format!("{FAMILY_PAGE}/parentalcontrols/{}", m.steam_id));
            }
        }
    }

    /// The Cancel button on a pending invitation.
    pub fn invite_actions(&mut self, ui: &mut Ui, t: &Theme, status: &FamilyStatus, family: &Family, steam_id: SteamId) {
        if status.role == FamilyRole::Adult && w::button(ui, t, None, t!("Cancel invite"), w::Kind::Ghost, w::Size::Sm).clicked() {
            self.open(Dialog::CancelInvite { family: family.id, steam_id });
        }
    }

    // ---------- the dialogs ----------

    /// The open dialog, if any.
    pub fn dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, status: &FamilyStatus) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut send = None;
        let mut close = false;
        let width = t.dense(500.0).min(ui.ctx().content_rect().width() - t.spacing.xl * 2.0);
        // Tall dialogs scroll in a short window, so the buttons stay reachable.
        let tallest = (ui.ctx().content_rect().height() - t.spacing.xl * 4.0).max(t.dense(200.0));
        let frame = egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(t.spacing.lg as i8));
        egui::Modal::new(egui::Id::unique("family-dialog")).frame(frame).show(ui.ctx(), |ui| {
            ui.set_width(width);
            egui::ScrollArea::vertical().id_salt("family-dialog-body").max_height(tallest).show(ui, |ui| {
                ui.set_width(width - t.spacing.sm);
                ui.spacing_mut().item_spacing.y = t.spacing.sm;
                match &mut dialog {
                    Dialog::Invite { family, who, role } => self.invite_dialog(ui, t, ctx, status, *family, who, role, &mut send, &mut close),
                    Dialog::CancelInvite { family, steam_id } => {
                        let name = ctx.state.persona(*steam_id).display_name();
                        ui.label(w::heading_m(t, t!("Cancel {name}'s invitation?", name = name), t.text));
                        rule(ui, t, t!("They won't be able to accept it. No cooldown starts, and you can invite them again."), t.text_dim);
                        let action = FamilyAction::CancelInvite { family: *family, steam_id: *steam_id };
                        self.buttons(ui, t, t!("Cancel invite"), w::Kind::Danger, true, action, &mut send, &mut close);
                    }
                    Dialog::Decline(i) => {
                        let name = ctx.state.persona(i.inviter).display_name();
                        ui.label(w::heading_m(t, t!("Decline {name}'s invitation?", name = name), t.text));
                        rule(ui, t, t!("You won't join this family. No cooldown starts."), t.text_dim);
                        let action = FamilyAction::Decline { family: i.family };
                        self.buttons(ui, t, t!("Decline"), w::Kind::Danger, true, action, &mut send, &mut close);
                    }
                    Dialog::Accept(i) => {
                        let name = ctx.state.persona(i.inviter).display_name();
                        ui.label(w::heading_m(t, t!("Join {name}'s family?", name = name), t.text));
                        ui.label(w::body_m(t, t!("You'd join as: {role}", role = role_name(i.role)), t.text));
                        rule(ui, t, t!("Once you accept, you can't join a different Steam Family for one year, even if you leave."), t.away);
                        rule(ui, t, t!("Only join people you know and trust, in your household."), t.text_dim);
                        if i.role == FamilyRole::Child {
                            child_warning(ui, t);
                        }
                        if status.cooldown > 0 {
                            rule(ui, t, &t!("Your account has a waiting time until {date} ({days} days). Steam only lets you in before that if this was your last family.", date = date_in(status.cooldown), days = days(status.cooldown)), t.away);
                        }
                        rule(ui, t, t!("Steam may ask you to confirm in the Steam Mobile app or by email."), t.text_dim);
                        self.tick(ui, t, t!("I understand I can't join another family for a year"));
                        let action = FamilyAction::Accept { family: i.family };
                        let ok = self.understood;
                        self.buttons(ui, t, t!("Accept and join"), w::Kind::Primary, ok, action, &mut send, &mut close);
                    }
                    Dialog::Remove { family, member } => {
                        let name = ctx.state.persona(member.steam_id).display_name();
                        ui.label(w::heading_m(t, t!("Remove {name} from the family?", name = name), t.text));
                        rule(ui, t, t!("Their games leave the family library, and they can't borrow this family's games."), t.text_dim);
                        if member.cooldown > 0 {
                            rule(ui, t, &t!("They can't join another family until {date} ({days} days).", date = date_in(member.cooldown), days = days(member.cooldown)), t.away);
                        }
                        rule(ui, t, t!("Their place can stay closed to new members for up to a year. They can still be invited back."), t.away);
                        if member.role == FamilyRole::Child {
                            rule(ui, t, t!("This child's limits may stop applying once they're out of the family."), t.away);
                        }
                        self.tick(ui, t, t!("I understand this can lock them out of a family for up to a year"));
                        let action = FamilyAction::Remove { family: *family, steam_id: member.steam_id };
                        let ok = self.understood;
                        self.buttons(ui, t, t!("Remove from family"), w::Kind::Danger, ok, action, &mut send, &mut close);
                    }
                    Dialog::Leave { family, cooldown } => {
                        ui.label(w::heading_m(t, t!("Leave this family?"), t.text));
                        rule(ui, t, t!("Your games leave the family library, and you can't borrow this family's games."), t.text_dim);
                        if *cooldown > 0 {
                            rule(ui, t, &t!("You can't join another family until {date} ({days} days).", date = date_in(*cooldown), days = days(*cooldown)), t.away);
                        }
                        rule(ui, t, t!("Your place can stay closed to new members for up to a year. You can rejoin this family if an adult invites you."), t.away);
                        self.tick(ui, t, t!("I understand this can lock me out of a family for up to a year"));
                        let action = FamilyAction::Leave { family: *family };
                        let ok = self.understood;
                        self.buttons(ui, t, t!("Leave family"), w::Kind::Danger, ok, action, &mut send, &mut close);
                    }
                    Dialog::Waiting { action, outcome, invite } => self.waiting_dialog(ui, t, action, *outcome, invite.as_ref(), &mut send, &mut close),
                }
            });
        });
        if !self.busy && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        // `send` and `answered` may replace it; otherwise it stays open.
        self.dialog = Some(dialog);
        if close {
            self.close();
        } else if let Some(action) = send {
            self.send(ctx, action);
        }
    }

    fn tick(&mut self, ui: &mut Ui, t: &Theme, label: &str) {
        ui.add_space(t.spacing.xs);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm;
            w::checkbox(ui, t, &mut self.understood, t.dense(20.0));
            let r = ui.add(egui::Label::new(w::body_m(t, label, t.text)).wrap().sense(Sense::click()));
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.understood = !self.understood;
            }
        });
    }

    /// The error (if any), then the action and "Not now".
    #[allow(clippy::too_many_arguments)]
    fn buttons(&self, ui: &mut Ui, t: &Theme, label: &str, kind: w::Kind, enabled: bool, action: FamilyAction, send: &mut Option<FamilyAction>, close: &mut bool) {
        if let Some(e) = &self.error {
            wrapped(ui, w::body_m(t, e, t.danger));
        }
        ui.add_space(t.spacing.sm);
        ui.horizontal(|ui| {
            if self.busy {
                ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
            } else if ui.add_enabled_ui(enabled, |ui| w::button(ui, t, None, label, kind, w::Size::Md)).inner.clicked() {
                *send = Some(action);
            }
            if w::button(ui, t, None, t!("Not now"), w::Kind::Ghost, w::Size::Md).clicked() {
                *close = true;
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn invite_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, status: &FamilyStatus, family: u64, who: &mut Option<SteamId>, role: &mut FamilyRole, send: &mut Option<FamilyAction>, close: &mut bool) {
        ui.label(w::heading_m(t, t!("Invite to your family"), t.text));
        let former = status.family.as_ref().map(|f| f.former.clone()).unwrap_or_default();
        if let Some(InviteBlock::SlotCooldown(s)) = status.invite_block() {
            wrapped(ui, w::body_m(t, t!("You can't invite new members for {days} more days. Former members can still be invited back.", days = days(s)), t.away));
        }
        // Your friends and former members who can be invited now.
        let mut people: Vec<model::Persona> = ctx.state.friend_list();
        for id in &former {
            if !people.iter().any(|p| p.steam_id == *id) {
                people.push(ctx.state.persona(*id));
            }
        }
        people.retain(|p| status.can_invite(p.steam_id));
        people.sort_by_key(|p| (!former.contains(&p.steam_id), p.display_name().to_lowercase()));
        let width = ui.available_width();
        w::search(ui, t, &mut self.search, t!("Search your friends"), width);
        let q = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().id_salt("family-invite-people").max_height(t.dense(200.0)).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            let mut shown = 0;
            for p in people.iter().filter(|p| q.is_empty() || p.display_name().to_lowercase().contains(&q)) {
                shown += 1;
                let note = if former.contains(&p.steam_id) { t!("Former member") } else { "" };
                let on = *who == Some(p.steam_id);
                if person_row(ui, t, &art::avatar(p.avatar_hash.as_deref()), &p.display_name(), note, on) {
                    *who = Some(p.steam_id);
                }
            }
            if shown == 0 {
                ui.label(w::body_m(t, t!("No one to invite. You can only invite Steam friends and former members."), t.text_faint));
            }
        });
        ui.horizontal(|ui| {
            ui.label(w::body_m(t, t!("Invite as"), t.text_dim));
            for r in [FamilyRole::Adult, FamilyRole::Child] {
                if w::chip(ui, t, role_name(r), *role == r).clicked() && *role != r {
                    *role = r;
                    self.understood = false;
                }
            }
        });
        rule(ui, t, t!("Once they accept, they can't join a different Steam Family for one year."), t.away);
        rule(ui, t, t!("Steam may ask you to confirm in the Steam Mobile app or by email before it sends the invitation."), t.text_dim);
        let child = *role == FamilyRole::Child;
        if child {
            child_warning(ui, t);
            self.tick(ui, t, t!("I'm inviting my own child, and I understand what this gives adults control of"));
        }
        let ok = who.is_some() && (!child || self.understood);
        let action = FamilyAction::Invite { family, steam_id: who.unwrap_or(0), role: *role };
        self.buttons(ui, t, t!("Send invitation"), w::Kind::Primary, ok, action, send, close);
    }

    /// Steam wants a confirmation on your phone or by email.
    #[allow(clippy::too_many_arguments)]
    fn waiting_dialog(&mut self, ui: &mut Ui, t: &Theme, action: &FamilyAction, outcome: FamilyOutcome, invite: Option<&FamilyInvite>, send: &mut Option<FamilyAction>, close: &mut bool) {
        let joining = matches!(action, FamilyAction::Accept { .. } | FamilyAction::ConfirmJoin { .. });
        ui.label(w::heading_m(t, if joining { t!("Confirm joining the family") } else { t!("Confirm the invitation") }, t.text));
        match (outcome, joining) {
            (FamilyOutcome::ConfirmOnPhone, true) => wrapped(ui, w::body_m(t, t!("Steam sent a request to your Steam Mobile app. Approve it there to join this family."), t.text)),
            (FamilyOutcome::ConfirmOnPhone, false) => wrapped(ui, w::body_m(t, t!("Steam sent a request to your Steam Mobile app. Approve it there to send this invitation."), t.text)),
            (FamilyOutcome::ConfirmByEmail, false) => wrapped(ui, w::body_m(t, t!("Steam sent you an email. Click the link in it to send this invitation."), t.text)),
            (FamilyOutcome::AlreadyAccepted, _) => wrapped(ui, w::body_m(t, t!("A confirmation request has already been sent. Approve it to join this family."), t.text)),
            (FamilyOutcome::ConfirmOther, _) => {
                wrapped(ui, w::body_m(t, t!("Steam wants another confirmation that this app can't show. Finish it on Steam's site."), t.text));
                if ui.link(w::body_m(t, t!("Open Steam's family page"), t.accent)).clicked() {
                    web(FAMILY_PAGE);
                }
            }
            _ => {}
        }
        // Joining by email: Steam sends a code to paste here.
        let code_step = joining && outcome == FamilyOutcome::ConfirmByEmail;
        if code_step {
            if invite.is_some_and(|i| i.role == FamilyRole::Child) {
                child_warning(ui, t);
            }
            wrapped(ui, w::body_m(t, t!("Steam emailed you a confirmation code. Enter it to join this family."), t.text));
            let width = ui.available_width();
            w::text_input(ui, t, &mut self.code, t!("Confirmation code"), width, 1);
        }
        if let Some(e) = &self.error {
            wrapped(ui, w::body_m(t, e, t.danger));
        }
        ui.add_space(t.spacing.sm);
        ui.horizontal(|ui| {
            if code_step {
                let code = parse_code(&self.code);
                if self.busy {
                    ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
                } else if ui.add_enabled_ui(code.is_some() && invite.is_some(), |ui| w::button(ui, t, None, t!("Confirm"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    if let (Some(code), Some(i)) = (code, invite) {
                        *send = Some(FamilyAction::ConfirmJoin { family: i.family, invite_id: i.invite_id, code });
                    }
                }
            }
            if w::button(ui, t, None, t!("Close"), w::Kind::Ghost, w::Size::Md).clicked() {
                *close = true;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_codes_are_numbers() {
        assert_eq!(parse_code(" 123456\n"), Some(123456));
        assert_eq!(parse_code("12a"), None);
        assert_eq!(parse_code(""), None);
        assert_eq!(parse_code("-5"), None);
        assert_eq!(parse_code("99999999999999999999999"), None);
    }

    #[test]
    fn the_demo_never_claims_a_real_answer_for_a_lockout_step() {
        // Accepting and inviting stop at a confirmation step in the demo.
        assert_eq!(demo_outcome(&FamilyAction::Accept { family: 1 }), FamilyOutcome::ConfirmByEmail);
        assert_eq!(demo_outcome(&FamilyAction::Invite { family: 1, steam_id: 2, role: FamilyRole::Adult }), FamilyOutcome::ConfirmOnPhone);
        assert_eq!(demo_outcome(&FamilyAction::Leave { family: 1 }), FamilyOutcome::Done);
    }

    #[test]
    fn days_round_up() {
        assert_eq!((days(1), days(86_400), days(86_401)), (1, 1, 2));
    }
}

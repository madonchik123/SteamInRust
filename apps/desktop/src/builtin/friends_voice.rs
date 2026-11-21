//! Voice chat in the Friends & Chat window: friends calling you, the call
//! you're in (who's talking, mute, deafen, volume per person, hang up), the
//! call buttons in conversations, and the voice settings.

use super::Friends;
use client::Command;
use egui::{Align, Align2, Color32, Layout, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::voice::{AudioDevice, Call, CallPhase, CallTarget, Participant, VoiceOp, VoiceSettings};
use model::{ChatGroup, SteamId, art};
use ui::{Theme, icons, widgets as w};

fn send(ctx: &Ctx, op: VoiceOp) {
    ctx.send(Command::Voice(op));
}

fn find_group(state: &client::State, group: u64) -> Option<&ChatGroup> {
    state.chat_groups.as_ref()?.as_ref().ok()?.iter().find(|g| g.id == group)
}

/// "Ferris" or "Lounge · Odyssey of Friends".
fn call_title(state: &client::State, target: CallTarget) -> String {
    match target {
        CallTarget::Friend(id) => state.persona(id).display_name(),
        CallTarget::Room { group, room } => {
            let Some(g) = find_group(state, group) else { return t!("Voice channel").to_string() };
            let channel = g.rooms.iter().find(|r| r.id == room).map(|r| r.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| t!("Voice channel").to_string());
            format!("{channel} · {}", super::group_name(state, g))
        }
    }
}

/// How tall the voice area at the bottom of the friends list is.
pub(crate) fn dock_height(state: &client::State, t: &Theme) -> f32 {
    let rings = state.voice.incoming.len().min(3) as f32 * t.dense(84.0);
    let call = state.voice.call.as_ref().map_or(0.0, |c| t.dense(if c.mic_problem.is_some() || c.speaker_problem.is_some() { 186.0 } else { 164.0 }));
    if rings + call > 0.0 { rings + call + t.spacing.md * 2.0 } else { 0.0 }
}

impl Friends {
    /// Friends calling you and the call you're in, under the friends list.
    pub fn voice_dock(&mut self, ui: &mut Ui, ctx: &mut Ctx, rect: Rect) {
        let t = ctx.theme.clone();
        ui.painter().rect_filled(rect, 0.0, t.surface);
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, t.surface_hover));
        let inner = rect.shrink2(vec2(t.spacing.lg, t.spacing.md));
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        for from in ctx.state.voice.incoming.clone().into_iter().take(3) {
            incoming_row(&mut ui, &t, ctx, from);
        }
        if let Some(call) = ctx.state.voice.call.clone() {
            call_panel(&mut ui, &t, ctx, &call);
        }
    }

    /// The call button at the top of a chat with a friend.
    pub(super) fn call_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, friend: SteamId) {
        let here = ctx.state.voice.call.as_ref().is_some_and(|c| c.target == CallTarget::Friend(friend));
        let (icon, tip) = if here { (icons::PHONE_DISCONNECT, t!("Hang up")) } else { (icons::PHONE, t!("Start a voice call")) };
        if w::icon_button(ui, t, icon, tip, here, 0).clicked() {
            send(ctx, if here { VoiceOp::Leave } else { VoiceOp::CallFriend(friend) });
        }
    }

    /// Join or leave the voice channel of the room that's open.
    pub(super) fn room_voice_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, group: u64, room: u64) {
        let target = CallTarget::Room { group, room };
        let here = ctx.state.voice.call.as_ref().is_some_and(|c| c.target == target);
        let r = if here { w::button(ui, t, Some(icons::PHONE_DISCONNECT), t!("Leave voice"), w::Kind::Danger, w::Size::Sm) } else { w::button(ui, t, Some(icons::MICROPHONE), t!("Join voice"), w::Kind::Primary, w::Size::Sm) };
        if r.clicked() {
            send(ctx, if here { VoiceOp::Leave } else { VoiceOp::JoinRoom { group, room } });
        }
    }
}

fn incoming_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, from: SteamId) {
    let p = ctx.state.persona(from);
    let name = p.display_name();
    ui.horizontal(|ui| {
        let size = t.dense(36.0);
        w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), size, Some(crate::shell::status_color(t, &p)));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(egui::Label::new(super::strong(t, &name, t.text)).truncate());
            ui.add(egui::Label::new(w::body_s(t, t!("Calling you…"), t.online)).truncate());
        });
    });
    ui.add_space(t.spacing.xs);
    ui.horizontal(|ui| {
        if w::button(ui, t, Some(icons::PHONE), t!("Answer"), w::Kind::Primary, w::Size::Sm).clicked() {
            send(ctx, VoiceOp::Accept(from));
        }
        if w::button(ui, t, Some(icons::PHONE_DISCONNECT), t!("Decline"), w::Kind::Danger, w::Size::Sm).clicked() {
            send(ctx, VoiceOp::Decline(from));
        }
    });
    ui.add_space(t.spacing.sm);
}

fn phase_text(call: &Call) -> String {
    match call.phase {
        CallPhase::Connecting => t!("Connecting…").to_string(),
        CallPhase::Ringing => t!("Ringing…").to_string(),
        CallPhase::InCall if call.people.len() <= 1 => t!("Only you're here").to_string(),
        CallPhase::InCall => t!("{n} people in voice", n = call.people.len()),
    }
}

fn call_panel(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, call: &Call) {
    let title = call_title(ctx.state, call.target);
    let live = call.phase == CallPhase::InCall;
    ui.horizontal(|ui| {
        let color = if live { t.online } else { t.away };
        ui.label(egui::RichText::new(icons::MICROPHONE).font(egui::FontId::proportional(t.dense(16.0))).color(color));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(egui::Label::new(super::strong(t, &title, t.text)).truncate());
            let settings = &ctx.state.voice_settings;
            let line = if live && settings.push_to_talk && !call.muted { format!("{} · {}", phase_text(call), t!("Hold {key} to talk", key = key_label(&settings.push_to_talk_key))) } else { phase_text(call) };
            ui.add(egui::Label::new(w::body_s(t, line, color)).truncate());
        });
    });
    for problem in [(&call.mic_problem, t!("No microphone: others can't hear you")), (&call.speaker_problem, t!("No speakers: you can't hear others"))] {
        if let (Some(why), text) = problem {
            ui.add(egui::Label::new(w::body_s(t, text, t.danger)).truncate()).on_hover_text(why);
        }
    }
    ui.add_space(t.spacing.sm);
    people_row(ui, t, ctx, call);
    ui.add_space(t.spacing.sm);
    ui.horizontal(|ui| {
        let (icon, tip) = if call.muted { (icons::MICROPHONE_SLASH, t!("Unmute")) } else { (icons::MICROPHONE, t!("Mute")) };
        if w::icon_button(ui, t, icon, tip, call.muted, 0).clicked() {
            send(ctx, VoiceOp::SetMuted(!call.muted));
        }
        let (icon, tip) = if call.deaf { (icons::SPEAKER_SLASH, t!("Turn sound back on")) } else { (icons::SPEAKER_HIGH, t!("Deafen")) };
        if w::icon_button(ui, t, icon, tip, call.deaf, 0).clicked() {
            send(ctx, VoiceOp::SetDeaf(!call.deaf));
        }
        if w::icon_button(ui, t, icons::GEAR, t!("Voice settings"), false, 0).clicked() {
            ctx.navigate("settings", Some("voice".into()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if matches!(call.target, CallTarget::Friend(_)) { t!("Hang up") } else { t!("Leave voice") };
            if w::button(ui, t, Some(icons::PHONE_DISCONNECT), label, w::Kind::Danger, w::Size::Sm).clicked() {
                send(ctx, VoiceOp::Leave);
            }
        });
    });
}

/// Everyone in the call: a ring while they talk, a mark when they're muted,
/// and a click for their volume.
fn people_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, call: &Call) {
    let size = t.dense(36.0);
    egui::ScrollArea::horizontal().id_salt("voice-people").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).max_height(size + 6.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm;
            for p in &call.people {
                person(ui, t, ctx, p, size, call);
            }
        });
    });
}

fn person(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, who: &Participant, size: f32, call: &Call) {
    let me = who.steam_id == ctx.state.steam_id;
    let persona = ctx.state.persona(who.steam_id);
    let (slot, resp) = ui.allocate_exact_size(vec2(size + 4.0, size + 4.0), if me { Sense::hover() } else { Sense::click() });
    let rect = slot.shrink(2.0);
    w::paint_presence_avatar(ui, t, rect, &art::avatar(persona.avatar_hash.as_deref()), None);
    let radius = t.radius_of(&t.components.friend_avatar);
    if who.talking {
        ui.painter().rect_stroke(rect.expand(1.0), radius, Stroke::new(2.0, t.online), StrokeKind::Outside);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    let (muted, deaf) = if me { (call.muted, call.deaf) } else { (who.muted || who.no_mic, who.deaf) };
    if muted || deaf {
        let badge = Rect::from_center_size(rect.right_bottom() - vec2(3.0, 3.0), vec2(15.0, 15.0));
        ui.painter().circle_filled(badge.center(), 7.5, t.danger);
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, if deaf { icons::SPEAKER_SLASH } else { icons::MICROPHONE_SLASH }, egui::FontId::proportional(10.0), Color32::WHITE);
    }
    let name = if me { t!("You").to_string() } else { persona.display_name() };
    let status = match (who.talking, muted, deaf) {
        (_, _, true) => t!("Sound off").to_string(),
        (_, true, _) => t!("Muted").to_string(),
        (true, ..) => t!("Talking").to_string(),
        _ => String::new(),
    };
    let tip = if status.is_empty() { name.clone() } else { format!("{name} · {status}") };
    let resp = resp.on_hover_text(tip);
    if me {
        return;
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(t.dense(210.0));
        ui.label(super::strong(t, &name, t.text));
        ui.label(w::body_s(t, t!("Volume for this person"), t.text_dim));
        let current = ctx.state.voice_settings.volume_of(who.steam_id);
        let mut percent = (current * 100.0).round();
        if ui.add(egui::Slider::new(&mut percent, 0.0..=400.0).suffix("%").step_by(5.0)).changed() {
            send(ctx, VoiceOp::SetPersonVolume(who.steam_id, percent / 100.0));
        }
        ui.horizontal(|ui| {
            let silenced = current <= 0.0;
            if w::button(ui, t, Some(if silenced { icons::SPEAKER_HIGH } else { icons::SPEAKER_SLASH }), if silenced { t!("Unmute for me") } else { t!("Mute for me") }, w::Kind::Secondary, w::Size::Sm).clicked() {
                send(ctx, VoiceOp::SetPersonVolume(who.steam_id, if silenced { 1.0 } else { 0.0 }));
            }
            if (current - 1.0).abs() > 0.005 && w::button(ui, t, None, t!("Reset"), w::Kind::Ghost, w::Size::Sm).clicked() {
                send(ctx, VoiceOp::SetPersonVolume(who.steam_id, 1.0));
            }
        });
    });
}

/// A key's name for people, from its saved name.
pub(crate) fn key_label(key: &str) -> String {
    match key {
        "Mouse4" => t!("Mouse button 4").to_string(),
        "Mouse5" => t!("Mouse button 5").to_string(),
        "MouseMiddle" => t!("Middle mouse button").to_string(),
        "LeftCtrl" => t!("Left Ctrl").to_string(),
        "RightCtrl" => t!("Right Ctrl").to_string(),
        "LeftShift" => t!("Left Shift").to_string(),
        "RightShift" => t!("Right Shift").to_string(),
        "LeftAlt" => t!("Left Alt").to_string(),
        "RightAlt" => t!("Right Alt").to_string(),
        "Space" => t!("Space").to_string(),
        "CapsLock" => "Caps Lock".to_string(),
        "Backquote" => "`".to_string(),
        k => k.to_string(),
    }
}

/// The saved name for a key egui reports.
pub(crate) fn key_name(key: egui::Key) -> Option<String> {
    let name = match key {
        egui::Key::Space => "Space".to_string(),
        egui::Key::Tab => "Tab".to_string(),
        egui::Key::Backtick => "Backquote".to_string(),
        k => k.name().to_uppercase(),
    };
    client::voice::ptt::known(&name).then_some(name)
}

/// The key or mouse button just pressed, while choosing one.
fn pressed_key(ui: &Ui) -> Option<String> {
    ui.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => {
                    if let Some(name) = key_name(*key) {
                        return Some(name);
                    }
                }
                egui::Event::PointerButton { button: egui::PointerButton::Extra1, pressed: true, .. } => return Some("Mouse4".to_string()),
                egui::Event::PointerButton { button: egui::PointerButton::Extra2, pressed: true, .. } => return Some("Mouse5".to_string()),
                egui::Event::PointerButton { button: egui::PointerButton::Middle, pressed: true, .. } => return Some("MouseMiddle".to_string()),
                _ => {}
            }
        }
        if i.modifiers.ctrl {
            Some("LeftCtrl".to_string())
        } else if i.modifiers.shift {
            Some("LeftShift".to_string())
        } else if i.modifiers.alt {
            Some("LeftAlt".to_string())
        } else {
            None
        }
    })
}

fn device_options(list: &[AudioDevice], chosen: &Option<String>) -> Vec<(Option<String>, String)> {
    let mut options = vec![(None, t!("System default").to_string())];
    options.extend(list.iter().map(|d| (Some(d.id.clone()), d.name.clone())));
    // A saved device that isn't plugged in stays visible instead of blank.
    if let Some(id) = chosen
        && !list.iter().any(|d| &d.id == id)
    {
        options.push((Some(id.clone()), t!("Not connected").to_string()));
    }
    options
}

fn volume_slider(ui: &mut Ui, value: &mut f32) -> bool {
    let mut percent = (*value * 100.0).round();
    let changed = ui.add(egui::Slider::new(&mut percent, 0.0..=400.0).suffix("%").step_by(5.0)).changed();
    if changed {
        *value = percent / 100.0;
    }
    changed
}

/// Settings > Friends & Chat: microphone, speakers, volumes, push-to-talk.
pub(crate) fn settings(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    use crate::builtin::settings::{group, row};
    let asked = egui::Id::unique("voice-devices-asked");
    if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
        ui.data_mut(|d| d.insert_temp(asked, true));
        send(ctx, VoiceOp::ListDevices);
    }
    let capturing = egui::Id::unique("voice-ptt-capture");
    let mut s: VoiceSettings = ctx.state.voice_settings.clone();
    let (inputs, outputs) = ctx.state.voice_devices.clone().unwrap_or_default();
    let mut changed = false;
    let width = t.dense(280.0);
    let (input_options, output_options) = (device_options(&inputs, &s.input), device_options(&outputs, &s.output));
    group(ui, t, t!("Voice chat"), |ui| {
        row(
            ui,
            t,
            t!("Microphone"),
            "",
            |ui| changed |= w::select(ui, t, &mut s.input, &input_options, width),
            |ui| {
                if w::icon_button(ui, t, icons::REFRESH, t!("Look for devices again"), false, 0).clicked() {
                    send(ctx, VoiceOp::ListDevices);
                }
            },
        );
        row(ui, t, t!("Speakers or headphones"), "", |ui| changed |= w::select(ui, t, &mut s.output, &output_options, width), |_| {});
        row(ui, t, t!("Microphone volume"), "", |ui| changed |= volume_slider(ui, &mut s.input_volume), |_| {});
        row(ui, t, t!("Voice volume"), t!("How loud other people are, all together"), |ui| changed |= volume_slider(ui, &mut s.output_volume), |_| {});
        row(ui, t, t!("Push to talk"), "", |ui| changed |= w::toggle(ui, t, &mut s.push_to_talk).changed(), |ui| {
            ui.add(egui::Label::new(w::body_m(t, t!("Others hear you only while you hold the key, even when another app is in front"), t.text_dim)).wrap());
        });
        if s.push_to_talk {
            let waiting = ui.data(|d| d.get_temp::<bool>(capturing).unwrap_or(false));
            row(
                ui,
                t,
                t!("Push-to-talk key"),
                "",
                |ui| {
                    let label = if waiting { t!("Press a key or mouse button…").to_string() } else { key_label(&s.push_to_talk_key) };
                    if w::button(ui, t, None, &label, w::Kind::Secondary, w::Size::Sm).clicked() {
                        ui.data_mut(|d| d.insert_temp(capturing, !waiting));
                    }
                },
                |_| {},
            );
            if waiting {
                ui.ctx().request_repaint();
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ui.data_mut(|d| d.insert_temp(capturing, false));
                } else if let Some(key) = pressed_key(ui) {
                    s.push_to_talk_key = key;
                    changed = true;
                    ui.data_mut(|d| d.insert_temp(capturing, false));
                }
            }
        }
    });
    if changed {
        send(ctx, VoiceOp::SetSettings(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_saved_names() {
        assert_eq!(key_name(egui::Key::A).as_deref(), Some("A"));
        assert_eq!(key_name(egui::Key::Num7).as_deref(), Some("7"));
        assert_eq!(key_name(egui::Key::F13).as_deref(), Some("F13"));
        assert_eq!(key_name(egui::Key::Space).as_deref(), Some("Space"));
        assert_eq!(key_name(egui::Key::Backtick).as_deref(), Some("Backquote"));
        assert_eq!(key_name(egui::Key::Escape), None);
    }

    #[test]
    fn a_missing_device_stays_listed() {
        let list = [AudioDevice { id: "a".into(), name: "Mic".into() }];
        assert_eq!(device_options(&list, &None).len(), 2);
        let o = device_options(&list, &Some("gone".into()));
        assert_eq!(o.len(), 3);
        assert_eq!(o[2].0.as_deref(), Some("gone"));
    }

    #[test]
    fn dock_is_only_there_when_needed() {
        let t = Theme::default();
        let mut state = client::State::default();
        assert_eq!(dock_height(&state, &t), 0.0);
        state.voice.incoming = vec![1];
        assert!(dock_height(&state, &t) > 0.0);
    }
}

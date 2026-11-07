//! Community: friends and 1:1 chat.

use i18n::t;
use client::{Client, Command, State};
use egui::{Align, Align2, CornerRadius, Key, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use model::{PersonaState, SteamId, art};
use std::collections::HashSet;
use ui::{Theme, icons, widgets as w};

#[derive(Default)]
pub struct Community {
    selected: Option<SteamId>,
    filter: String,
    input: String,
    loaded: HashSet<SteamId>,
    /// Friends page: which section, its search, the "add a friend" box.
    tab: FriendsTab,
    search: String,
    add: String,
    remove_confirm: Option<SteamId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum FriendsTab {
    #[default]
    Friends,
    Add,
    Invites,
    Blocked,
}

impl Extension for Community {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "community",
            name: "Community",
            author: "SteamInRust",
            description: "Friends and chat",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("community", "Community", icons::USERS).main().order(30)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        if let Some(id) = arg.and_then(|a| a.parse().ok()) {
            self.selected = Some(id);
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        if let Some(id) = self.selected {
            if self.loaded.insert(id) {
                ctx.send(Command::LoadHistory { friend: id });
            }
            if ctx.state.unread.contains_key(&id) {
                ctx.act(Action::MarkRead(id));
            }
        }

        egui::Panel::left("community-friends")
            .exact_size(300.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 16, right: 10, top: 18, bottom: 10 }))
            .show(ui, |ui| {
                ui.label(w::text_bold(t!("Friends"), 20.0, t.text));
                ui.add_space(8.0);
                let width = ui.available_width() - 4.0;
                w::search(ui, &t, &mut self.filter, t!("Search friends"), width);
                ui.add_space(10.0);
                if let Some(id) = friend_list(ui, &t, ctx.state, &self.filter, self.selected) {
                    self.selected = Some(id);
                }
            });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| match self.selected {
            Some(id) => self.conversation(ui, &t, ctx, id),
            None => self.overview(ui, &t, ctx),
        });
    }
}

impl Community {
    fn conversation(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        let p = ctx.state.persona(id);
        egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(24, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 44.0, Some(crate::shell::status_color(t, &p)));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(3.0);
                    ui.label(w::text_bold(if p.name.is_empty() { id.to_string() } else { p.name.clone() }, 18.0, t.text));
                    let r = ui.add(egui::Label::new(w::text(crate::shell::status_text(ctx.state, &p), 13.0, crate::shell::status_color(t, &p))).sense(Sense::click()));
                    if let Some(game) = p.game_app_id.filter(|_| p.in_game()) {
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            ctx.act(crate::shell::game_page(ctx.state, game));
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::COPY, t!("Copy SteamID"), false, 0).clicked() {
                        ui.ctx().copy_text(id.to_string());
                        ctx.toast(t!("SteamID copied"));
                    }
                    if w::icon_button(ui, t, icons::X, t!("Close conversation"), false, 0).clicked() {
                        self.selected = None;
                    }
                });
            });
        });
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0, t.border));
        egui::Frame::new().inner_margin(Margin::symmetric(24, 12)).show(ui, |ui| {
            let h = ui.available_height();
            chat_body(ui, t, ctx.state, &client_of(ctx), id, &mut self.input, h);
        });
    }
}

/// Extensions reach the client through `Ctx::send`; chat_body takes a
/// `Client` so the overlay can share it. This adapter wraps `Ctx`.
fn client_of<'a, 'b>(ctx: &'a Ctx<'b>) -> CtxSender<'a, 'b> {
    CtxSender(ctx)
}

pub struct CtxSender<'a, 'b>(&'a Ctx<'b>);

pub trait Sender {
    fn send(&self, cmd: Command);
}
impl Sender for CtxSender<'_, '_> {
    fn send(&self, cmd: Command) {
        self.0.send(cmd);
    }
}
impl Sender for Client {
    fn send(&self, cmd: Command) {
        Client::send(self, cmd);
    }
}

/// Message list plus composer. Used by Community and the overlay.
pub fn chat_body(ui: &mut Ui, t: &Theme, state: &State, client: &impl Sender, friend: SteamId, input: &mut String, height: f32) {
    let empty = Vec::new();
    let msgs = state.chats.get(&friend).unwrap_or(&empty);
    // The composer goes in first, at the bottom, and the messages get exactly
    // what's left, so the whole thing never needs more than `height` (inside a
    // resizable window, needing more made the window grow every frame).
    let size = egui::vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.set_max_size(size);
        egui::Frame::new()
            .fill(t.raised)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(t.corner())
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let before = input.len();
                    let r = ui.add(
                        egui::TextEdit::singleline(input)
                            .hint_text(w::text(t!("Message {name}", name = state.persona(friend).name), 14.0, t.text_faint))
                            .font(w::font(14.5))
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width() - 48.0),
                    );
                    if input.len() > before && before == 0 {
                        client.send(Command::Typing { to: friend });
                    }
                    let send = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let clicked = w::icon_button(ui, t, icons::SEND, t!("Send"), false, 0).clicked();
                    if (send || clicked) && !input.trim().is_empty() {
                        client.send(Command::SendMessage { to: friend, text: input.trim().to_string() });
                        input.clear();
                        r.request_focus();
                    }
                });
            });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("chat", friend))
                .max_height(ui.available_height().max(40.0))
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if msgs.is_empty() {
                        w::empty(ui, t, icons::CHAT, t!("No messages yet"), t!("Say hi!"));
                    }
                    let mut last: Option<(SteamId, u64)> = None;
                    for m in msgs {
                        let new_group = last.is_none_or(|(from, time)| from != m.from || m.time.saturating_sub(time) > 300);
                        if new_group {
                            ui.add_space(10.0);
                            let p = state.persona(m.from);
                            ui.horizontal(|ui| {
                                w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 34.0, None);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        let me = m.from == state.steam_id;
                                        let name = if p.name.is_empty() { t!("You").to_string() } else { p.name.clone() };
                                        ui.label(w::text_semibold(name, 14.0, if me { t.accent } else { t.text }));
                                        ui.label(w::text(format_time(m.time), 11.5, t.text_faint));
                                    });
                                    ui.add(egui::Label::new(w::text(&m.text, 14.0, t.text)).wrap());
                                });
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.add_space(42.0);
                                ui.add(egui::Label::new(w::text(&m.text, 14.0, t.text)).wrap());
                            });
                        }
                        last = Some((m.from, m.time));
                    }
                    if state.is_typing(friend) {
                        ui.add_space(6.0);
                        ui.label(w::text(t!("{name} is typing…", name = state.persona(friend).name), 12.5, t.text_faint).italics());
                        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    }
                });
        });
    });
}

fn format_time(unix: u64) -> String {
    if unix == 0 {
        return String::new();
    }
    ui::ago(unix)
}

/// Compact friend list for the Community sidebar.
fn friend_list(ui: &mut Ui, t: &Theme, state: &State, filter: &str, selected: Option<SteamId>) -> Option<SteamId> {
    let f = filter.to_lowercase();
    let mut list = state.friend_list();
    list.retain(|p| f.is_empty() || p.name.to_lowercase().contains(&f));
    // Unread first, then in game, online, offline.
    list.sort_by_key(|p| {
        let unread = state.unread.get(&p.steam_id).copied().unwrap_or(0);
        (unread == 0, !p.in_game(), p.state == PersonaState::Offline, p.name.to_lowercase())
    });
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("community-list").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            ui.label(w::text(t!("No friends to show"), 13.0, t.text_faint));
        }
        for p in &list {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            if selected == Some(p.steam_id) {
                ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
            } else if hover > 0.0 {
                ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface, hover));
            }
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 6.0))).layout(Layout::left_to_right(Align::Center)));
            w::avatar(&mut c, t, &art::avatar(p.avatar_hash.as_deref()), 36.0, Some(crate::shell::status_color(t, p)));
            c.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(2.0);
                let off = p.state == PersonaState::Offline;
                ui.add(egui::Label::new(w::text_medium(&p.name, 14.0, if off { t.text_faint } else { t.text })).truncate());
                let status = if state.is_typing(p.steam_id) {
                    "typing…".to_string()
                } else if let Some(m) = state.chats.get(&p.steam_id).and_then(|c| c.last()) {
                    m.text.clone()
                } else {
                    crate::shell::status_text(state, p)
                };
                ui.add(egui::Label::new(w::text(status, 12.0, t.text_dim)).truncate());
            });
            if let Some(n) = state.unread.get(&p.steam_id).filter(|n| **n > 0) {
                let cc = rect.right_center() - vec2(16.0, 0.0);
                ui.painter().circle_filled(cc, 9.0, t.accent);
                ui.painter().text(cc, egui::Align2::CENTER_CENTER, n.min(&99).to_string(), w::font_bold(10.0), t.on_accent);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(p.steam_id);
            }
        }
    });
    clicked
}

impl Community {
    /// The friends page, laid out like Steam's: a menu on the left and the
    /// friends in a grid, grouped by in game / online / offline.
    fn overview(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let state = ctx.state;
        let count = |rel: model::Relationship| state.friends.values().filter(|r| **r == rel).count();
        let (friends_n, invites_n) = (count(model::Relationship::Friend), count(model::Relationship::RequestRecipient));
        let blocked_n = state.friends.values().filter(|r| matches!(r, model::Relationship::Blocked | model::Relationship::Ignored)).count();
        egui::Frame::new().inner_margin(Margin::same(28)).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Left menu.
                ui.allocate_ui_with_layout(vec2(220.0, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(220.0);
                    ui.label(w::text_bold(t!("FRIENDS"), 11.5, t.text_faint));
                    ui.add_space(4.0);
                    let items = [
                        (FriendsTab::Friends, icons::USER, t!("Your friends"), friends_n),
                        (FriendsTab::Add, icons::USER_PLUS, t!("Add a friend"), 0),
                        (FriendsTab::Invites, icons::BELL, t!("Invites"), invites_n),
                        (FriendsTab::Blocked, icons::LOCK_SIMPLE, t!("Blocked"), blocked_n),
                    ];
                    for (tab, icon, label, n) in items {
                        let (r, resp) = ui.allocate_exact_size(vec2(220.0, 36.0), Sense::click());
                        let on = self.tab == tab;
                        let hover = resp.hovered();
                        if on || hover {
                            ui.painter().rect_filled(r, t.corner_small(), if on { t.accent.gamma_multiply(0.16) } else { t.surface_hover });
                        }
                        ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::CENTER_CENTER, icon, w::font(15.0), if on { t.accent } else { t.text_dim });
                        ui.painter().text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, label, w::font_medium(13.5), if on { t.text } else { t.text_dim });
                        if n > 0 {
                            let highlight = tab == FriendsTab::Invites;
                            let txt = n.to_string();
                            let g = ui.painter().layout_no_wrap(txt, w::font_bold(11.0), if highlight { t.on_accent } else { t.text_faint });
                            let pill = Rect::from_center_size(pos2(r.right() - 10.0 - g.size().x / 2.0 - 4.0, r.center().y), g.size() + vec2(10.0, 4.0));
                            if highlight {
                                ui.painter().rect_filled(pill, CornerRadius::same(8), t.accent);
                            }
                            ui.painter().galley(pill.center() - g.size() / 2.0, g, t.text);
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                ui.add_space(24.0);
                // Main area.
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("friends-page").auto_shrink([false, false]).show(ui, |ui| match self.tab {
                        FriendsTab::Friends => self.friends_grid(ui, t, ctx, friends_n),
                        FriendsTab::Add => self.add_friend(ui, t, ctx),
                        FriendsTab::Invites => self.invites(ui, t, ctx),
                        FriendsTab::Blocked => self.blocked(ui, t, ctx),
                    });
                });
            });
        });
        self.remove_dialog(ui, t, ctx);
    }

    fn friends_grid(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, total: usize) {
        ui.horizontal(|ui| {
            ui.label(w::text_bold(t!("YOUR FRIENDS"), 13.0, t.text));
            ui.label(w::text_bold(total.to_string(), 13.0, t.accent));
        });
        ui.add_space(8.0);
        w::search(ui, t, &mut self.search, &t!("Search friends by name or game"), 420.0);
        ui.add_space(14.0);
        let q = self.search.trim().to_lowercase();
        let all: Vec<model::Persona> = ctx
            .state
            .friend_list()
            .into_iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.game_name.as_deref().unwrap_or_default().to_lowercase().contains(&q))
            .collect();
        let in_game: Vec<_> = all.iter().filter(|p| p.in_game()).cloned().collect();
        let online: Vec<_> = all.iter().filter(|p| !p.in_game() && p.state != PersonaState::Offline).cloned().collect();
        let offline: Vec<_> = all.iter().filter(|p| !p.in_game() && p.state == PersonaState::Offline).cloned().collect();
        if all.is_empty() {
            ui.label(w::text(t!("No friends match."), 13.5, t.text_faint));
        }
        for (title, list) in [(t!("IN GAME"), in_game), (t!("ONLINE"), online), (t!("OFFLINE"), offline)] {
            if list.is_empty() {
                continue;
            }
            ui.label(w::text_bold(format!("{title}  ·  {}", list.len()), 11.5, t.text_faint));
            ui.add_space(6.0);
            let card = vec2(290.0, 52.0);
            let gap = 10.0;
            let cols = (((ui.available_width() + gap) / (card.x + gap)).floor() as usize).max(1);
            for row in list.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for p in row {
                        self.friend_card(ui, t, ctx, p, card);
                    }
                });
                ui.add_space(gap);
            }
            ui.add_space(10.0);
        }
    }

    fn friend_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, p: &model::Persona, size: egui::Vec2) {
        let (r, resp) = ui.allocate_exact_size(size, Sense::click());
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        let color = crate::shell::status_color(t, p);
        let offline = p.state == PersonaState::Offline;
        ui.painter().rect_filled(r, CornerRadius::same(4), w::mix(t.surface, t.surface_hover, hover));
        let av = Rect::from_min_size(r.min, vec2(size.y, size.y));
        if !w::paint_cover(ui, av, &art::avatar(p.avatar_hash.as_deref()), CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }) {
            w::skeleton(ui, t, av, CornerRadius::same(4));
        }
        if offline {
            ui.painter().rect_filled(av, CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, egui::Color32::from_black_alpha(110));
        }
        // Status stripe, like Steam's.
        ui.painter().rect_filled(Rect::from_min_size(pos2(av.right(), r.top()), vec2(3.0, size.y)), CornerRadius::ZERO, color);
        let x = av.right() + 12.0;
        let max_w = r.right() - x - 8.0;
        let fit = |text: String, font: egui::FontId, c: egui::Color32| {
            let mut job = egui::text::LayoutJob::simple_singleline(text, font, c);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
            ui.painter().layout_job(job)
        };
        let name_color = if p.in_game() { t.in_game } else if offline { t.text_dim } else { t.accent };
        let name = fit(p.name.clone(), w::font_semibold(14.0), name_color);
        let status = fit(crate::shell::status_text(ctx.state, p), w::font(12.0), if offline { t.text_faint } else { color });
        ui.painter().galley(pos2(x, r.top() + 8.0), name, name_color);
        ui.painter().galley(pos2(x, r.top() + 28.0), status, color);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            ctx.act(Action::OpenChat(p.steam_id));
        }
        resp.context_menu(|ui| {
            if ui.button(format!("{}   {}", icons::CHAT, t!("Send a message"))).clicked() {
                ctx.act(Action::OpenChat(p.steam_id));
                ui.close();
            }
            if ui.button(format!("{}   {}", icons::USER_CIRCLE, t!("View profile"))).clicked() {
                ctx.navigate("profile", Some(p.steam_id.to_string()));
                ui.close();
            }
            if ui.button(format!("{}   {}", icons::BACKPACK, t!("Inventory"))).clicked() {
                ctx.navigate("inventory", Some(p.steam_id.to_string()));
                ui.close();
            }
            ui.separator();
            if ui.button(format!("{}   {}", icons::X, t!("Remove friend…"))).clicked() {
                self.remove_confirm = Some(p.steam_id);
                ui.close();
            }
        });
    }

    fn remove_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(id) = self.remove_confirm else { return };
        let name = ctx.state.persona(id).name;
        let mut done = false;
        egui::Modal::new(egui::Id::unique(("remove-friend", id))).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.label(w::text_bold(t!("Remove {name} from friends?", name = name), 17.0, t.text));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::X), t!("Remove friend"), w::Kind::Danger, w::Size::Md).clicked() {
                    ctx.send(client::Command::RemoveFriend { steam_id: id });
                    done = true;
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    done = true;
                }
            });
        });
        if done {
            self.remove_confirm = None;
        }
    }

    fn add_friend(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.label(w::text_bold(t!("Add a friend"), 20.0, t.text));
        ui.add_space(10.0);
        let code = (ctx.state.steam_id & 0xFFFF_FFFF).to_string();
        w::panel(t).show(ui, |ui| {
            ui.set_width(ui.available_width().min(620.0));
            ui.label(w::text_bold(t!("YOUR FRIEND CODE"), 11.5, t.text_faint));
            ui.horizontal(|ui| {
                ui.label(w::text_bold(&code, 24.0, t.text));
                if w::button(ui, t, Some(icons::COPY), t!("Copy"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    ui.ctx().copy_text(code.clone());
                    ctx.toast(t!("Friend code copied"));
                }
            });
            ui.label(w::text(t!("Share it with a friend so they can add you."), 12.5, t.text_dim));
        });
        ui.add_space(12.0);
        w::panel(t).show(ui, |ui| {
            ui.set_width(ui.available_width().min(620.0));
            ui.label(w::text_bold(t!("ADD BY FRIEND CODE OR PROFILE LINK"), 11.5, t.text_faint));
            ui.add_space(4.0);
            let target = parse_friend(&self.add);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.add).hint_text(t!("e.g. 836957442 or steamcommunity.com/profiles/…")).desired_width(380.0).margin(vec2(10.0, 8.0)));
                ui.add_enabled_ui(target.is_some(), |ui| {
                    if w::button(ui, t, Some(icons::USER_PLUS), t!("Send invite"), w::Kind::Primary, w::Size::Md).clicked() {
                        if let Some(id) = target {
                            ctx.send(client::Command::AddFriend { steam_id: id });
                            ctx.toast(t!("Friend invite sent"));
                            self.add.clear();
                        }
                    }
                });
            });
            if !self.add.trim().is_empty() && target.is_none() {
                ui.label(w::text(t!("Enter a friend code (numbers) or a link like steamcommunity.com/profiles/7656…"), 12.0, t.away));
            }
        });
    }

    fn invites(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let pick = |rel: model::Relationship| -> Vec<model::Persona> { ctx.state.friends.iter().filter(|(_, r)| **r == rel).map(|(id, _)| ctx.state.persona(*id)).collect() };
        let received = pick(model::Relationship::RequestRecipient);
        let sent = pick(model::Relationship::RequestInitiator);
        ui.label(w::text_bold(t!("Invites"), 20.0, t.text));
        ui.add_space(10.0);
        if received.is_empty() && sent.is_empty() {
            ui.label(w::text(t!("No pending invites."), 13.5, t.text_faint));
        }
        for (title, list, incoming) in [(t!("RECEIVED"), received, true), (t!("SENT"), sent, false)] {
            if list.is_empty() {
                continue;
            }
            ui.label(w::text_bold(format!("{title}  ·  {}", list.len()), 11.5, t.text_faint));
            ui.add_space(6.0);
            for p in &list {
                egui::Frame::new().fill(t.surface).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width().min(720.0));
                    ui.horizontal(|ui| {
                        w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 36.0, None);
                        let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
                        ui.label(w::text_semibold(name, 14.0, t.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if incoming {
                                if w::button(ui, t, Some(icons::X), t!("Ignore"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                    ctx.send(client::Command::RemoveFriend { steam_id: p.steam_id });
                                }
                                if w::button(ui, t, Some(icons::CHECK), t!("Accept"), w::Kind::Primary, w::Size::Sm).clicked() {
                                    ctx.send(client::Command::AddFriend { steam_id: p.steam_id });
                                }
                            } else if w::button(ui, t, Some(icons::X), t!("Cancel invite"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                ctx.send(client::Command::RemoveFriend { steam_id: p.steam_id });
                            }
                            if w::button(ui, t, Some(icons::USER_CIRCLE), t!("Profile"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                ctx.navigate("profile", Some(p.steam_id.to_string()));
                            }
                        });
                    });
                });
                ui.add_space(6.0);
            }
            ui.add_space(10.0);
        }
    }

    fn blocked(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.label(w::text_bold(t!("Blocked"), 20.0, t.text));
        ui.label(w::text(t!("They can't message you or send invites."), 13.0, t.text_dim));
        ui.add_space(10.0);
        let list: Vec<model::Persona> = ctx.state.friends.iter().filter(|(_, r)| matches!(r, model::Relationship::Blocked | model::Relationship::Ignored)).map(|(id, _)| ctx.state.persona(*id)).collect();
        if list.is_empty() {
            ui.label(w::text(t!("You haven't blocked anyone."), 13.5, t.text_faint));
        }
        for p in &list {
            ui.horizontal(|ui| {
                w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 32.0, None);
                let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
                ui.label(w::text_semibold(name, 13.5, t.text));
                if ui.link(w::text(t!("Profile"), 12.5, t.accent)).clicked() {
                    ctx.navigate("profile", Some(p.steam_id.to_string()));
                }
            });
            ui.add_space(4.0);
        }
    }
}

/// A friend code (account id), SteamID64, or profile link -> SteamID64.
fn parse_friend(input: &str) -> Option<SteamId> {
    let s = input.trim().trim_end_matches('/');
    let s = s.rsplit_once("/profiles/").map(|x| x.1).unwrap_or(s);
    let n: u64 = s.trim_end_matches('/').parse().ok()?;
    match n {
        1..=0xFFFF_FFFF => Some(n + 76561197960265728),
        76561197960265729..=76561202255233023 => Some(n),
        _ => None,
    }
}

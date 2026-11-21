//! Store: getting a Points Shop item with your points, inside the app.
//! Your balance, a large preview (items that move play), whether you own
//! it, a confirmation with the item and its cost, then an offer to wear it.
//!
//! Points are only ever spent from the confirmation's button, at the price
//! it showed, once: nothing here retries. `--demo` never talks to Steam
//! about any of this: the balance, what you own and the outcome are made up.

use super::Store;
use client::Command;
use client::points::{OwnedReward, RedeemFail};
use egui::{Align, Align2, Color32, Layout, Rect, Sense, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, ProfileSave, SteamId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use store::points::{RewardItem, class};
use ui::{Theme, icons, widgets as w};

use super::points::kind_name;
use super::thousands;

/// How often the balance is read again while the shop is open.
const BALANCE_EVERY: Duration = Duration::from_secs(60);
/// The demo account's points.
const DEMO_BALANCE: i64 = 12_500;

/// What can be done about an item right now.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Avail {
    /// You own it (its community item id; 0 if Steam hasn't said yet).
    Owned(u64),
    Buy,
    /// This many points short.
    Short(u64),
    /// Steam only offers it to people with its game.
    NeedsGame,
    /// A bundle or the like: Steam's page handles those.
    OnSteamOnly,
    /// Still finding out (the buy button stays off until it's known).
    Waiting,
    /// Couldn't find out.
    Unknown(String),
    SignedOut,
}

/// What can be done about `item`. Buying is only offered once the balance,
/// what you own and (where it matters) your eligible games are all known,
/// so nothing can be bought twice or on a guess.
pub(super) fn availability(
    item: &RewardItem,
    signed_in: bool,
    balance: Option<&Result<i64, String>>,
    owned: Option<Result<Option<u64>, String>>,
    eligible: Option<&Result<HashSet<AppId>, String>>,
) -> Avail {
    if !signed_in {
        return Avail::SignedOut;
    }
    if !item.redeemable_here() {
        return Avail::OnSteamOnly;
    }
    match owned {
        None => return Avail::Waiting,
        Some(Err(e)) => return Avail::Unknown(e),
        Some(Ok(Some(id))) => return Avail::Owned(id),
        Some(Ok(None)) => {}
    }
    let points = match balance {
        None => return Avail::Waiting,
        Some(Err(e)) => return Avail::Unknown(e.clone()),
        Some(Ok(p)) => *p,
    };
    if item.needs_eligible_game() {
        match eligible {
            None => return Avail::Waiting,
            Some(Err(e)) => return Avail::Unknown(e.clone()),
            Some(Ok(games)) if !games.contains(&item.app_id) => return Avail::NeedsGame,
            Some(Ok(_)) => {}
        }
    }
    let have = points.max(0) as u64;
    if have < item.cost { Avail::Short(item.cost - have) } else { Avail::Buy }
}

/// Where the item's window is.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Look,
    /// "Spend N points on this?"
    Confirm,
    Redeeming,
    Got,
    Failed(RedeemFail),
    Equipping(u64),
    Equipped,
    EquipFailed(u64, String),
}

struct Dialog {
    item: RewardItem,
    step: Step,
    player: Option<video::Player>,
}

/// Your points and Points Shop items.
#[derive(Default)]
pub(super) struct Wallet {
    /// The account these are for.
    account: SteamId,
    balance: Option<Result<i64, String>>,
    balance_asked: Option<Instant>,
    eligible: Option<Result<HashSet<AppId>, String>>,
    /// (game, item type) → community item id.
    owned: HashMap<(AppId, u32), u64>,
    /// Got here this session (Steam's inventory can lag a moment behind).
    got: HashMap<(AppId, u32), u64>,
    /// Games whose items were asked about / have been answered / failed.
    asked: HashSet<AppId>,
    known: HashSet<AppId>,
    failed: HashMap<AppId, String>,
    dialog: Option<Dialog>,
}

impl Wallet {
    fn owned_state(&self, it: &RewardItem) -> Option<Result<Option<u64>, String>> {
        let key = (it.app_id, it.item_type);
        if let Some(id) = self.got.get(&key).or_else(|| self.owned.get(&key)) {
            return Some(Ok(Some(*id)));
        }
        if self.known.contains(&it.app_id) {
            return Some(Ok(None));
        }
        self.failed.get(&it.app_id).map(|e| Err(e.clone()))
    }

    /// True if you own it, for the marks on the cards.
    pub(super) fn owns(&self, it: &RewardItem) -> bool {
        matches!(self.owned_state(it), Some(Ok(Some(_))))
    }

    /// It's unclear whether points were just spent on this game's item:
    /// nothing can be bought from it until Steam's numbers are read again.
    fn forget(&mut self, app: AppId) {
        self.balance = None;
        self.balance_asked = None;
        self.known.remove(&app);
        self.asked.remove(&app);
        self.failed.remove(&app);
    }

    fn take_inventory(&mut self, apps: &[AppId], items: &[OwnedReward]) {
        self.owned.retain(|(a, _), _| !apps.contains(a));
        self.owned.extend(items.iter().map(|i| ((i.app_id, i.item_type), i.id)));
        for a in apps {
            self.known.insert(*a);
            self.failed.remove(a);
        }
    }
}

/// A plain sentence for each way Steam can refuse.
fn fail_text(f: &RedeemFail) -> String {
    match f {
        RedeemFail::NotEnoughPoints => t!("You don't have enough points for this item.").to_string(),
        RedeemFail::PriceChanged => t!("This item's price changed, so nothing was spent. Check the new price and try again.").to_string(),
        RedeemFail::Duplicate => t!("Steam is already handling this request. Check your points before trying again.").to_string(),
        RedeemFail::NoLongerAvailable => t!("This item isn't available any more.").to_string(),
        RedeemFail::NotAllowed => t!("Steam didn't allow this on your account.").to_string(),
        RedeemFail::NoPrivileges => t!("Your account can't spend points right now.").to_string(),
        RedeemFail::NoAnswer => t!("Steam didn't answer, so your points may or may not have been spent. Check your balance and items before trying again.").to_string(),
        RedeemFail::Other(m) => t!("Steam refused: {error}", error = i18n::tr_dyn(m)),
    }
}

fn equip_label(kind: u8) -> &'static str {
    match kind {
        0 => t!("Use as profile background"),
        1 => t!("Use as mini-profile background"),
        2 => t!("Use as avatar frame"),
        _ => t!("Use as avatar"),
    }
}

/// What a click in the item's window asked for.
enum Act {
    None,
    Close,
    ToConfirm,
    Back,
    Spend,
    Equip(u8, u64),
    Retry,
    Profile,
}

impl Store {
    /// Keep the balance and what you own up to date for the items shown.
    pub(super) fn wallet_tick(&mut self, ctx: &mut Ctx, items: &[RewardItem]) {
        let me = ctx.state.steam_id;
        if self.points.wallet.account != me {
            self.points.wallet = Wallet { account: me, ..Default::default() };
        }
        if me == 0 {
            return;
        }
        if crate::app::demo() {
            let owned_games: HashSet<AppId> = self.owned.iter().copied().collect();
            let wl = &mut self.points.wallet;
            wl.balance.get_or_insert(Ok(DEMO_BALANCE));
            wl.eligible.get_or_insert(Ok(owned_games));
            for it in items {
                wl.known.insert(it.app_id);
                // A few made-up items already on the account.
                if it.defid % 7 == 0 {
                    wl.owned.insert((it.app_id, it.item_type), it.defid);
                }
            }
            return;
        }
        let wl = &mut self.points.wallet;
        if wl.balance_asked.is_none_or(|a| a.elapsed() >= BALANCE_EVERY) {
            wl.balance_asked = Some(Instant::now());
            ctx.send(Command::LoadPoints);
        }
        let new: Vec<AppId> = items.iter().map(|i| i.app_id).filter(|a| wl.asked.insert(*a)).collect();
        for apps in new.chunks(50) {
            ctx.send(Command::LoadRewardInventory { apps: apps.to_vec() });
        }
    }

    pub(super) fn wallet_event(&mut self, event: &client::Event) {
        let wl = &mut self.points.wallet;
        match event {
            client::Event::PointsBalance(r) => {
                // A failed refresh doesn't hide a number already known.
                if r.is_ok() || !matches!(wl.balance, Some(Ok(_))) {
                    wl.balance = Some(r.clone());
                }
            }
            client::Event::RewardEligible(r) => {
                if r.is_ok() || !matches!(wl.eligible, Some(Ok(_))) {
                    wl.eligible = Some(r.clone().map(|v| v.into_iter().collect()));
                }
            }
            client::Event::RewardInventory { apps, result } => match result {
                Ok(items) => wl.take_inventory(apps, items),
                Err(e) => {
                    for a in apps.iter().filter(|a| !wl.known.contains(a)) {
                        wl.failed.insert(*a, e.clone());
                    }
                }
            },
            client::Event::PointsRedeemed { defid, result } => {
                let result = result.clone();
                self.redeem_finished(*defid, result);
            }
            client::Event::ProfileSaved { what: ProfileSave::Item, result } => {
                if let Some(d) = &mut wl.dialog {
                    if let Step::Equipping(id) = d.step {
                        d.step = match result {
                            Ok(()) => Step::Equipped,
                            Err(e) => Step::EquipFailed(id, e.clone()),
                        };
                    }
                }
            }
            _ => {}
        }
    }

    fn redeem_finished(&mut self, defid: u32, result: Result<u64, RedeemFail>) {
        let wl = &mut self.points.wallet;
        let Some(d) = wl.dialog.as_mut().filter(|d| d.item.defid == defid as u64 && d.step == Step::Redeeming) else {
            // No window waiting for it: the balance and inventory Steam
            // sends right after this still bring the app up to date.
            return;
        };
        match result {
            Ok(id) => {
                wl_got(&mut wl.got, &mut wl.owned, &d.item, id);
                if let Some(Ok(p)) = &mut wl.balance {
                    *p -= d.item.cost as i64;
                }
                d.step = Step::Got;
            }
            Err(f) => {
                let app = d.item.app_id;
                let stale = f == RedeemFail::PriceChanged;
                let unsure = matches!(f, RedeemFail::NoAnswer | RedeemFail::Duplicate);
                d.step = Step::Failed(f);
                if unsure {
                    wl.forget(app);
                }
                if stale {
                    // Show Steam's current prices.
                    self.points.reload();
                }
            }
        }
    }

    /// Open an item's window.
    pub(super) fn points_open_item(&mut self, item: &RewardItem) {
        self.points.wallet.dialog = Some(Dialog { item: item.clone(), step: Step::Look, player: None });
    }

    /// The balance box in the shop's header.
    pub(super) fn balance_box(&self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        if ctx.state.steam_id == 0 {
            return;
        }
        let wl = &self.points.wallet;
        let (text, tip) = match &wl.balance {
            Some(Ok(p)) => (thousands((*p).max(0) as u64), t!("Your points").to_string()),
            Some(Err(e)) => ("—".to_string(), t!("Couldn't read your points: {error}", error = i18n::tr_dyn(e))),
            None => ("…".to_string(), t!("Your points").to_string()),
        };
        // One label: the header lays out right to left.
        let label: egui::WidgetText = w::heading_m(t, format!("{}  {text}", icons::COINS), t.accent).into();
        let galley = label.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
        let (mx, my) = (t.dense(20.0), t.dense(13.0));
        // Its own height, so a title wrapped to two lines doesn't stretch it.
        let size = galley.size() + vec2(2.0 * mx, 2.0 * my);
        ui.allocate_ui(size, |ui| {
            w::component_frame(t, &t.components.store_card)
                .inner_margin(egui::Margin::symmetric(mx as i8, my as i8))
                .show(ui, |ui| {
                    ui.label(galley);
                })
                .response
                .on_hover_text(tip);
        });
    }

    /// The item's window, while one is open.
    pub(super) fn points_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(mut d) = self.points.wallet.dialog.take() else { return };
        // Left the shop (Back, a link) with nothing under way: it's closed.
        if self.view != super::View::Points && !matches!(d.step, Step::Redeeming | Step::Equipping(_)) {
            return;
        }
        let screen = ui.ctx().content_rect();
        let width = (screen.width() - 2.0 * t.dense(48.0)).clamp(320.0, t.dense(880.0));
        let wide = width >= t.dense(640.0);
        let pic_w = if wide { (width * 0.5).floor() } else { width };
        // Backgrounds and movies are wide; avatars, frames and stickers square.
        let widescreen = matches!(d.item.class, class::BACKGROUND | class::MINI_BACKGROUND | class::STARTUP_MOVIE);
        let room = (screen.height() - t.dense(if wide { 150.0 } else { 420.0 })).max(120.0);
        let pic_h = (if widescreen { pic_w * 0.5625 } else { pic_w }).min(room).floor();
        let signed_in = ctx.state.steam_id != 0;
        let wl = &self.points.wallet;
        let avail = availability(&d.item, signed_in, wl.balance.as_ref(), wl.owned_state(&d.item), wl.eligible.as_ref());
        let balance = wl.balance.as_ref().and_then(|b| b.as_ref().ok()).copied();
        let mut act = Act::None;
        let rt = self.rt.clone();
        let modal = egui::Modal::new(egui::Id::unique("points-item")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            let layout = if wide { Layout::left_to_right(Align::Min) } else { Layout::top_down(Align::Min) };
            ui.with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing = vec2(t.dense(20.0), t.dense(14.0));
                let (rect, _) = ui.allocate_exact_size(vec2(pic_w, pic_h), Sense::hover());
                preview(ui, t, rect, &mut d, &rt);
                let col_w = if wide { width - pic_w - t.dense(20.0) } else { width };
                ui.allocate_ui_with_layout(vec2(col_w, pic_h), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(col_w);
                    ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.dense(8.0));
                    item_column(ui, t, &d, &avail, balance, &mut act);
                });
            });
        });
        let busy = matches!(d.step, Step::Redeeming | Step::Equipping(_));
        if !busy && modal.should_close() {
            act = Act::Close;
        }
        match act {
            Act::None => {}
            Act::Close => return,
            Act::ToConfirm if avail == Avail::Buy => d.step = Step::Confirm,
            Act::ToConfirm => {}
            Act::Back => d.step = Step::Look,
            // Only from the confirmation, and only while it's still affordable.
            Act::Spend if d.step == Step::Confirm && avail == Avail::Buy => {
                d.step = Step::Redeeming;
                let (defid, app_id, cost) = (d.item.defid as u32, d.item.app_id, d.item.cost);
                if crate::app::demo() {
                    self.points.wallet.dialog = Some(d);
                    self.redeem_finished(defid, demo_outcome(defid));
                    return;
                }
                ctx.send(Command::RedeemPoints { defid, app_id, cost });
            }
            Act::Spend => d.step = Step::Look,
            Act::Equip(kind, id) if id != 0 => {
                d.step = Step::Equipping(id);
                if crate::app::demo() {
                    d.step = Step::Equipped;
                } else {
                    ctx.send(Command::EquipProfileItem { kind, id });
                }
            }
            Act::Equip(..) => {}
            Act::Retry => {
                let wl = &mut self.points.wallet;
                wl.balance_asked = None;
                if matches!(wl.balance, Some(Err(_))) {
                    wl.balance = None;
                }
                if matches!(wl.eligible, Some(Err(_))) {
                    wl.eligible = None;
                }
                wl.failed.remove(&d.item.app_id);
                wl.asked.remove(&d.item.app_id);
                d.step = Step::Look;
            }
            Act::Profile => {
                ctx.navigate("profile", None);
                return;
            }
        }
        self.points.wallet.dialog = Some(d);
    }
}

fn wl_got(got: &mut HashMap<(AppId, u32), u64>, owned: &mut HashMap<(AppId, u32), u64>, item: &RewardItem, id: u64) {
    let key = (item.app_id, item.item_type);
    got.insert(key, id);
    owned.insert(key, id);
}

/// `--demo`: how a redeem turns out (`--demo-redeem-error=107` tries one of
/// Steam's refusals). Nothing is sent anywhere.
fn demo_outcome(defid: u32) -> Result<u64, RedeemFail> {
    match std::env::args().find_map(|a| a.strip_prefix("--demo-redeem-error=").and_then(|c| c.parse::<i32>().ok())) {
        Some(0) => Err(RedeemFail::NoAnswer),
        Some(code) => Err(RedeemFail::from_eresult(code, "")),
        None => Ok(defid as u64 + 900_000),
    }
}

/// The item, large: its video if it has one, else its animated picture,
/// else its picture.
fn preview(ui: &mut Ui, t: &Theme, rect: Rect, d: &mut Dialog, rt: &tokio::runtime::Handle) {
    ui.painter().rect_filled(rect, t.corner_small(), t.chrome);
    let it = &d.item;
    if !it.movie.is_empty() {
        let player = d.player.get_or_insert_with(|| {
            let opts = video::Options { autoplay: true, muted: true, volume: 0.0, looping: true, width: 960 };
            video::Player::new(rt, ui.ctx(), video::Source::Mp4(it.movie.clone()), opts)
        });
        player.poll(ui.ctx());
        if player.status() != video::Status::Failed {
            ui.ctx().request_repaint();
        }
        if let Some((tex, size)) = player.texture() {
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let target = Align2::CENTER_CENTER.align_size_within_rect(size * scale, rect);
            ui.painter().image(tex, target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            return;
        }
    } else if !it.gif.is_empty() {
        // egui plays a GIF's frames when it draws it from its address.
        let side = rect.width().min(rect.height());
        egui::Image::new(it.gif.as_str()).paint_at(ui, Rect::from_center_size(rect.center(), vec2(side, side)));
        return;
    }
    if it.image.is_empty() {
        if it.movie.is_empty() || video_failed(d) {
            w::placeholder(ui, t, rect, &it.name, t.corner_small());
        } else {
            w::skeleton(ui, t, rect, t.corner_small());
        }
    } else if !w::paint_contain(ui, rect, &it.image, Align2::CENTER_CENTER) {
        w::skeleton(ui, t, rect, t.corner_small());
    }
}

/// The item's video can't be shown here (the player only has H.264, and
/// some of Steam's videos, all startup movies among them, are H.265).
fn video_failed(d: &Dialog) -> bool {
    d.player.as_ref().is_some_and(|p| p.status() == video::Status::Failed && p.texture().is_none())
}

/// Says so, and where it can be watched before spending anything on it.
fn video_note(ui: &mut Ui, t: &Theme, d: &Dialog, avail: &Avail) {
    if !video_failed(d) {
        return;
    }
    ui.add_space(t.spacing.xs);
    note(ui, t, t!("This video can't play in the app. You can watch it on Steam's page."), t.text_dim);
    // These two already offer the same button.
    if !matches!(avail, Avail::NeedsGame | Avail::OnSteamOnly) {
        steam_page_button(ui, t, d.item.app_id);
    }
}

/// The price row: the coin and the cost.
fn cost_row(ui: &mut Ui, t: &Theme, cost: u64) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        ui.label(w::heading_m(t, icons::COINS, t.accent));
        ui.label(w::heading_m(t, thousands(cost), t.text));
    });
}

fn note(ui: &mut Ui, t: &Theme, text: &str, color: Color32) {
    ui.add(egui::Label::new(w::body_m(t, text, color)).wrap());
}

fn steam_page_button(ui: &mut Ui, t: &Theme, app: AppId) {
    if w::button(ui, t, Some(icons::EXTERNAL), t!("Open on Steam"), w::Kind::Ghost, w::Size::Md).on_hover_text(t!("Opens in your browser")).clicked() {
        crate::links::open_http(&format!("https://store.steampowered.com/points/shop/app/{app}"));
    }
}

/// The right side of the window: what it is, what it costs, what can be done.
fn item_column(ui: &mut Ui, t: &Theme, d: &Dialog, avail: &Avail, balance: Option<i64>, act: &mut Act) {
    let it = &d.item;
    let busy = matches!(d.step, Step::Redeeming | Step::Equipping(_));
    ui.horizontal_top(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if !busy && w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                *act = Act::Close;
            }
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.add(egui::Label::new(w::heading_m(t, &it.name, t.text)).wrap());
            });
        });
    });
    ui.label(w::body_m(t, kind_name(it.class), t.text_dim));
    if !it.description.is_empty() {
        ui.add(egui::Label::new(w::body_s(t, &it.description, t.text_dim)).wrap());
    }
    ui.add_space(t.spacing.xs);
    if !matches!(avail, Avail::Owned(_)) || matches!(d.step, Step::Confirm | Step::Redeeming) {
        cost_row(ui, t, it.cost);
    }
    if let Some(p) = balance {
        ui.label(w::body_m(t, t!("Your points: {n}", n = thousands(p.max(0) as u64)), t.text_dim));
    }
    ui.add_space(t.spacing.sm);
    match &d.step {
        Step::Look => {
            look_actions(ui, t, it, avail, act);
            video_note(ui, t, d, avail);
        }
        Step::Confirm => {
            let after = balance.unwrap_or(0).max(0) as u64;
            ui.add(egui::Label::new(w::body_l(t, t!("Spend {cost} points on {name}?", cost = thousands(it.cost), name = it.name), t.text)).wrap());
            note(ui, t, &t!("You'll have {n} points left.", n = thousands(after.saturating_sub(it.cost))), t.text_dim);
            ui.add_space(t.spacing.xs);
            ui.horizontal_wrapped(|ui| {
                if w::button(ui, t, Some(icons::COINS), &t!("Spend {n} points", n = thousands(it.cost)), w::Kind::Primary, w::Size::Md).clicked() {
                    *act = Act::Spend;
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    *act = Act::Back;
                }
            });
        }
        Step::Redeeming => spinner(ui, t, t!("Getting it from Steam…")),
        Step::Got => {
            ui.label(w::heading_s(t, format!("{}  {}", icons::CHECK, t!("It's yours!")), t.play));
            let id = if let Avail::Owned(id) = avail { *id } else { 0 };
            equip_actions(ui, t, it, id, act);
        }
        Step::Failed(f) => {
            note(ui, t, &fail_text(f), t.danger);
            ui.add_space(t.spacing.xs);
            ui.horizontal_wrapped(|ui| {
                // Trying again only after a clear "no" that a retry could change.
                if matches!(f, RedeemFail::Other(_)) && w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                    *act = Act::Retry;
                }
                if w::button(ui, t, None, t!("Close"), w::Kind::Secondary, w::Size::Md).clicked() {
                    *act = Act::Close;
                }
            });
        }
        Step::Equipping(_) => spinner(ui, t, t!("Putting it on your profile…")),
        Step::Equipped => {
            ui.label(w::heading_s(t, format!("{}  {}", icons::CHECK, t!("It's on your profile.")), t.play));
            ui.horizontal_wrapped(|ui| {
                if w::button(ui, t, Some(icons::USER), t!("See your profile"), w::Kind::Secondary, w::Size::Md).clicked() {
                    *act = Act::Profile;
                }
                if w::button(ui, t, None, t!("Close"), w::Kind::Ghost, w::Size::Md).clicked() {
                    *act = Act::Close;
                }
            });
        }
        Step::EquipFailed(id, e) => {
            note(ui, t, &t!("Steam didn't put it on your profile: {error}", error = i18n::tr_dyn(e)), t.danger);
            equip_actions(ui, t, it, *id, act);
        }
    }
}

fn spinner(ui: &mut Ui, t: &Theme, text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().color(t.accent));
        ui.label(w::body_m(t, text, t.text_dim));
    });
}

/// What the window offers before anything has been spent.
fn look_actions(ui: &mut Ui, t: &Theme, it: &RewardItem, avail: &Avail, act: &mut Act) {
    match avail {
        Avail::Buy => {
            if w::button(ui, t, Some(icons::COINS), &t!("Get for {n} points", n = thousands(it.cost)), w::Kind::Primary, w::Size::Md).clicked() {
                *act = Act::ToConfirm;
            }
        }
        Avail::Owned(id) => {
            ui.label(w::heading_s(t, format!("{}  {}", icons::CHECK, t!("You own this")), t.play));
            equip_actions(ui, t, it, *id, act);
        }
        Avail::Short(n) => {
            ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::COINS), &t!("Get for {n} points", n = thousands(it.cost)), w::Kind::Primary, w::Size::Md));
            note(ui, t, &t!("You need {n} more points.", n = thousands(*n)), t.away);
        }
        Avail::NeedsGame => {
            note(ui, t, t!("Steam only offers this item to people who have its game."), t.away);
            steam_page_button(ui, t, it.app_id);
        }
        Avail::OnSteamOnly => {
            note(ui, t, t!("Bundles and badges are only sold on Steam's Points Shop page."), t.text_dim);
            steam_page_button(ui, t, it.app_id);
        }
        Avail::Waiting => spinner(ui, t, t!("Checking your points and items…")),
        Avail::Unknown(e) => {
            note(ui, t, &t!("Couldn't check your points and items: {error}", error = i18n::tr_dyn(e)), t.danger);
            if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                *act = Act::Retry;
            }
        }
        Avail::SignedOut => note(ui, t, t!("Sign in to get items with your points."), t.text_dim),
    }
}

/// "Use as …" for items you can wear; `id` 0 means Steam hasn't said which
/// item it is yet.
fn equip_actions(ui: &mut Ui, t: &Theme, it: &RewardItem, id: u64, act: &mut Act) {
    let Some(kind) = it.equip_kind() else { return };
    if id != 0 && w::button(ui, t, Some(icons::USER_CIRCLE), equip_label(kind), w::Kind::Secondary, w::Size::Md).clicked() {
        *act = Act::Equip(kind, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> RewardItem {
        RewardItem { defid: 7, app_id: 620, class: class::AVATAR, reward_type: 1, item_type: 3, cost: 500, ..Default::default() }
    }

    fn avail(it: &RewardItem, balance: Option<Result<i64, String>>, owned: Option<Result<Option<u64>, String>>, eligible: Option<Result<HashSet<AppId>, String>>) -> Avail {
        availability(it, true, balance.as_ref(), owned, eligible.as_ref())
    }

    #[test]
    fn buying_needs_everything_known() {
        let it = item();
        assert_eq!(avail(&it, Some(Ok(500)), Some(Ok(None)), None), Avail::Buy);
        // Not yet known what you own or have: no buy button.
        assert_eq!(avail(&it, Some(Ok(500)), None, None), Avail::Waiting);
        assert_eq!(avail(&it, None, Some(Ok(None)), None), Avail::Waiting);
        assert_eq!(avail(&it, Some(Err("x".into())), Some(Ok(None)), None), Avail::Unknown("x".into()));
        assert_eq!(avail(&it, Some(Ok(500)), Some(Err("y".into())), None), Avail::Unknown("y".into()));
    }

    #[test]
    fn owned_items_are_not_for_sale_again() {
        assert_eq!(avail(&item(), Some(Ok(99_999)), Some(Ok(Some(42))), None), Avail::Owned(42));
    }

    #[test]
    fn short_of_points() {
        assert_eq!(avail(&item(), Some(Ok(499)), Some(Ok(None)), None), Avail::Short(1));
        assert_eq!(avail(&item(), Some(Ok(-5)), Some(Ok(None)), None), Avail::Short(500));
    }

    #[test]
    fn plain_backgrounds_need_the_game() {
        let it = RewardItem { class: class::BACKGROUND, ..item() };
        let yes: HashSet<AppId> = [620].into_iter().collect();
        assert_eq!(avail(&it, Some(Ok(500)), Some(Ok(None)), Some(Ok(yes))), Avail::Buy);
        assert_eq!(avail(&it, Some(Ok(500)), Some(Ok(None)), Some(Ok(HashSet::new()))), Avail::NeedsGame);
        assert_eq!(avail(&it, Some(Ok(500)), Some(Ok(None)), None), Avail::Waiting);
    }

    #[test]
    fn bundles_stay_on_steam_and_signed_out_cant_buy() {
        let bundle = RewardItem { reward_type: 6, ..item() };
        assert_eq!(avail(&bundle, Some(Ok(99_999)), Some(Ok(None)), None), Avail::OnSteamOnly);
        assert_eq!(availability(&item(), false, Some(&Ok(500)), Some(Ok(None)), None), Avail::SignedOut);
    }

    #[test]
    fn inventory_lag_cant_make_a_new_item_look_unowned() {
        let it = item();
        let mut w = Wallet::default();
        wl_got(&mut w.got, &mut w.owned, &it, 55);
        // Steam's list arrives without it yet.
        w.take_inventory(&[620], &[]);
        assert_eq!(w.owned_state(&it), Some(Ok(Some(55))));
        assert!(w.owns(&it));
    }

    #[test]
    fn after_no_answer_nothing_is_for_sale_until_steam_is_read_again() {
        let it = item();
        let mut w = Wallet { balance: Some(Ok(9_000)), ..Default::default() };
        w.take_inventory(&[620], &[]);
        assert_eq!(availability(&it, true, w.balance.as_ref(), w.owned_state(&it), None), Avail::Buy);
        w.forget(620);
        assert_eq!(availability(&it, true, w.balance.as_ref(), w.owned_state(&it), None), Avail::Waiting);
        // The fresh inventory alone isn't enough: the balance is needed too.
        w.take_inventory(&[620], &[OwnedReward { app_id: 620, item_type: 3, id: 5 }]);
        assert_eq!(availability(&it, true, w.balance.as_ref(), w.owned_state(&it), None), Avail::Owned(5));
    }

    #[test]
    fn inventory_marks_games_known() {
        let it = item();
        let mut w = Wallet::default();
        assert_eq!(w.owned_state(&it), None);
        w.take_inventory(&[620], &[OwnedReward { app_id: 620, item_type: 3, id: 9 }]);
        assert_eq!(w.owned_state(&it), Some(Ok(Some(9))));
        w.take_inventory(&[620], &[]);
        assert_eq!(w.owned_state(&it), Some(Ok(None)));
    }
}

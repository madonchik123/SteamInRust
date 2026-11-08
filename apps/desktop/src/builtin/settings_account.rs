//! Account pages inside Settings, read natively from Steam: account
//! details, purchase history, licenses and your Steam Family. They only
//! read; changing something (password, email, family members) still opens
//! Steam's site, where Steam asks to confirm it.

use super::settings::{info_row, web};
use client::Command;
use egui::{Color32, Margin, Rect, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{FamilyRole, art};
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AccountView {
    #[default]
    Home,
    Details,
    Purchases,
    Licenses,
}

#[derive(Default)]
pub struct AccountPages {
    pub view: AccountView,
    license_filter: String,
    shared_filter: String,
    /// Loads already asked for this visit: security, purchases, licenses, family.
    asked: [bool; 4],
    loading_more: bool,
}

fn language() -> String {
    i18n::language().steam_name().to_string()
}

fn date(unix: Option<u64>) -> String {
    unix.map(ui::date).unwrap_or_else(|| "—".into())
}

/// "Loading…", or the error with a Retry button. True when Retry is pressed.
fn not_ready<T>(ui: &mut Ui, t: &Theme, r: Option<&Result<T, String>>) -> bool {
    match r {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(w::text(t!("Loading…"), 13.5, t.text_dim));
            });
            false
        }
        Some(Err(e)) => {
            ui.label(w::text(t!("Steam didn't answer: {error}", error = e), 13.5, t.danger));
            ui.add_space(6.0);
            w::button(ui, t, Some(icons::REFRESH), t!("Retry"), w::Kind::Secondary, w::Size::Sm).clicked()
        }
        Some(Ok(_)) => false,
    }
}

fn section(ui: &mut Ui, t: &Theme, title: &str) {
    ui.add_space(14.0);
    ui.label(w::text_bold(title.to_uppercase(), 11.5, t.text_faint));
    ui.add_space(6.0);
}

/// Cells of one table row, left to right, with fixed widths (the last
/// takes what's left). Returns the row's rect.
fn row(ui: &mut Ui, t: &Theme, widths: &[f32], striped: bool, mut cell: impl FnMut(usize, &mut Ui)) -> Rect {
    let frame = egui::Frame::new().fill(if striped { t.surface } else { Color32::TRANSPARENT }).inner_margin(Margin::symmetric(10, 7)).corner_radius(t.corner_small());
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (i, wdt) in widths.iter().enumerate() {
                    let width = if i + 1 == widths.len() { ui.available_width().max(*wdt) } else { *wdt };
                    ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(width);
                        cell(i, ui);
                    });
                }
            });
        })
        .response
        .rect
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).wrap());
}

impl AccountPages {
    /// Back to the account overview, above each page.
    fn header(&mut self, ui: &mut Ui, t: &Theme, title: &str, subtitle: Option<&str>) {
        if w::button(ui, t, Some(icons::CARET_LEFT), t!("Account"), w::Kind::Ghost, w::Size::Sm).clicked() {
            self.view = AccountView::Home;
        }
        ui.add_space(4.0);
        w::page_title(ui, t, title, subtitle);
    }

    /// Opening a page asks Steam again (the data can change any time).
    pub fn open(&mut self, view: AccountView) {
        self.view = view;
        self.asked = [false; 4];
    }

    pub fn show(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        match self.view {
            AccountView::Home => {}
            AccountView::Details => self.details(ui, t, ctx),
            AccountView::Purchases => self.purchases(ui, t, ctx),
            AccountView::Licenses => self.licenses(ui, t, ctx),
        }
    }

    pub fn ask_security(&mut self, ctx: &mut Ctx) {
        if !self.asked[0] {
            self.asked[0] = true;
            ctx.send(Command::LoadAccountSecurity);
        }
    }

    fn details(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        self.header(ui, t, t!("Account details"), Some(t!("Your account's contact, security and sign-in details.")));
        self.ask_security(ctx);
        let security = ctx.state.account_security.clone();

        section(ui, t, t!("Contact"));
        let email = ctx.state.email.as_ref().map(|(e, ok)| if *ok { format!("{e}  ·  {}", t!("verified")) } else { format!("{e}  ·  {}", t!("not verified")) }).unwrap_or_else(|| "—".into());
        if info_row(ui, t, t!("Email address"), &email, Some(t!("Change email address"))) {
            web("https://help.steampowered.com/wizard/HelpChangeEmail");
        }
        let phone = match &security {
            Some(Ok(s)) if s.phone_verified => t!("A verified phone number is on your account").to_string(),
            Some(Ok(_)) => t!("No phone number").to_string(),
            _ => "…".into(),
        };
        if info_row(ui, t, t!("Phone number"), &phone, Some(t!("Manage phone number"))) {
            web("https://store.steampowered.com/phone/manage");
        }
        // The store country follows the wallet's currency, like Steam's page.
        let country = model::store_country(ctx.state.wallet.map(|(_, c)| c), &ctx.state.country).map(|c| c.to_uppercase()).unwrap_or_else(|| "—".into());
        info_row(ui, t, t!("Store country"), &country, None);

        section(ui, t, t!("Security"));
        if not_ready(ui, t, security.as_ref()) {
            self.asked[0] = false;
        }
        if let Some(Ok(s)) = &security {
            let guard = if s.authenticator {
                t!("Steam Guard Mobile Authenticator, since {date}", date = date(s.authenticator_since)).to_string()
            } else if s.steam_guard {
                t!("Codes by email, since {date}", date = date(s.steam_guard_since)).to_string()
            } else {
                t!("Off: your account isn't protected").to_string()
            };
            if info_row(ui, t, t!("Steam Guard"), &guard, Some(t!("Manage Steam Guard"))) {
                web("https://store.steampowered.com/twofactor/manage");
            }
            let password = match (s.password_changed, s.password_reset) {
                (Some(c), _) => t!("Last changed: {date}", date = date(Some(c))).to_string(),
                (None, Some(r)) => t!("Last reset: {date}", date = date(Some(r))).to_string(),
                _ => "—".to_string(),
            };
            if info_row(ui, t, t!("Password"), &password, Some(t!("Change password"))) {
                web("https://help.steampowered.com/wizard/HelpChangePassword");
            }
            if s.email_changed.is_some() {
                info_row(ui, t, t!("Email address last changed"), &date(s.email_changed), None);
            }
            if !s.devices.is_empty() {
                section(ui, t, t!("Computers Steam Guard remembers"));
                for d in &s.devices {
                    let name = if d.name.is_empty() { t!("Unnamed computer").to_string() } else { d.name.clone() };
                    let value = [date(d.since), d.ip.clone()].into_iter().filter(|s| !s.is_empty() && s != "—").collect::<Vec<_>>().join("  ·  ");
                    info_row(ui, t, &name, &value, None);
                }
            }
        }

        section(ui, t, t!("Steam account"));
        info_row(ui, t, t!("Account name"), &ctx.state.account, None);
        info_row(ui, t, t!("Steam ID"), &ctx.state.steam_id.to_string(), None);
        let balance = ctx.state.wallet.map(|(b, c)| model::format_money(b, c)).unwrap_or_else(|| "—".into());
        if info_row(ui, t, t!("Steam Wallet"), &balance, Some(t!("Purchase history"))) {
            self.open(AccountView::Purchases);
        }
        let vac = match ctx.state.profiles.get(&ctx.state.steam_id) {
            Some(p) if p.vac_banned => t!("VAC bans on record").to_string(),
            Some(_) => t!("No VAC bans").to_string(),
            None => "…".into(),
        };
        info_row(ui, t, t!("VAC status"), &vac, None);
    }

    fn purchases(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        self.header(ui, t, t!("Purchase history"), Some(t!("Store and in-game purchases, gifts, refunds, wallet and Market transactions.")));
        if !self.asked[1] {
            self.asked[1] = true;
            self.loading_more = false;
            ctx.send(Command::LoadPurchases { language: language(), cursor: None });
        }
        let page = ctx.state.purchases.clone();
        if not_ready(ui, t, page.as_ref()) {
            self.asked[1] = false;
        }
        let Some(Ok(page)) = page else { return };
        if page.rows.is_empty() {
            ui.label(w::text(t!("No purchases yet."), 14.0, t.text_dim));
            return;
        }
        const WIDTHS: [f32; 5] = [92.0, 260.0, 150.0, 96.0, 110.0];
        row(ui, t, &WIDTHS, false, |i, ui| {
            let h = [t!("DATE"), t!("ITEMS"), t!("TYPE"), t!("TOTAL"), t!("WALLET")][i];
            ui.label(w::text_bold(h, 10.5, t.text_faint));
        });
        for (n, p) in page.rows.iter().enumerate() {
            let dim = if p.refunded { t.text_faint } else { t.text };
            row(ui, t, &WIDTHS, n % 2 == 0, |i, ui| match i {
                0 => wrapped(ui, w::text(&p.date, 12.5, t.text_dim)),
                1 => {
                    let mut lines = p.items.iter();
                    if let Some(first) = lines.next() {
                        wrapped(ui, w::text_medium(first, 13.0, dim));
                    }
                    for l in lines {
                        wrapped(ui, w::text(l, 12.0, t.text_dim));
                    }
                }
                2 => {
                    wrapped(ui, w::text(&p.kind, 12.5, dim));
                    if !p.payment.is_empty() {
                        wrapped(ui, w::text(&p.payment, 11.5, t.text_faint));
                    }
                }
                3 => {
                    let mut total = w::text_medium(&p.total, 13.0, dim);
                    if p.refunded {
                        total = total.strikethrough();
                    }
                    wrapped(ui, total);
                }
                _ => {
                    if !p.wallet_change.is_empty() {
                        let up = p.wallet_change.starts_with('+');
                        wrapped(ui, w::text_medium(&p.wallet_change, 12.5, if up { t.play } else { t.text_dim }));
                    }
                    if !p.wallet_balance.is_empty() {
                        wrapped(ui, w::text(&p.wallet_balance, 11.5, t.text_faint));
                    }
                }
            });
        }
        ui.add_space(10.0);
        if let Some(cursor) = &page.more {
            if self.loading_more {
                ui.spinner();
            } else if w::button(ui, t, None, t!("Load more transactions"), w::Kind::Secondary, w::Size::Md).clicked() {
                self.loading_more = true;
                ctx.send(Command::LoadPurchases { language: language(), cursor: Some(cursor.clone()) });
            }
        }
        // A finished "load more" changes the cursor.
        if self.loading_more && ctx.state.purchases.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|p| p.more != page.more || p.rows.len() != page.rows.len()) {
            self.loading_more = false;
        }
        ui.add_space(6.0);
        ui.label(w::text(t!("For help with a purchase, Steam Support has a page per transaction."), 12.0, t.text_faint));
        if w::button(ui, t, Some(icons::QUESTION), t!("Steam Support"), w::Kind::Ghost, w::Size::Sm).clicked() {
            ctx.navigate("support", None);
        }
    }

    fn licenses(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        self.header(ui, t, t!("Licenses and product key activations"), Some(t!("Every game, DLC and package on your account, and how you got it.")));
        if !self.asked[2] {
            self.asked[2] = true;
            ctx.send(Command::LoadLicenses { language: language() });
        }
        let list = ctx.state.licenses.clone();
        if not_ready(ui, t, list.as_ref()) {
            self.asked[2] = false;
        }
        let Some(Ok(list)) = list else { return };
        ui.horizontal(|ui| {
            w::search(ui, t, &mut self.license_filter, t!("Search licenses"), 320.0);
            ui.add_space(12.0);
            ui.label(w::text(t!("{n} licenses", n = list.len()), 13.0, t.text_dim));
        });
        ui.add_space(8.0);
        let needle = self.license_filter.to_lowercase();
        const WIDTHS: [f32; 3] = [100.0, 440.0, 160.0];
        row(ui, t, &WIDTHS, false, |i, ui| {
            ui.label(w::text_bold([t!("DATE"), t!("ITEM"), t!("ACQUISITION METHOD")][i], 10.5, t.text_faint));
        });
        for (n, l) in list.iter().filter(|l| needle.is_empty() || l.name.to_lowercase().contains(&needle)).enumerate() {
            row(ui, t, &WIDTHS, n % 2 == 0, |i, ui| match i {
                0 => wrapped(ui, w::text(&l.date, 12.5, t.text_dim)),
                1 => wrapped(ui, w::text(&l.name, 13.0, t.text)),
                _ => wrapped(ui, w::text(&l.method, 12.5, t.text_dim)),
            });
        }
    }

    pub fn family(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::page_title(ui, t, t!("Family"), Some(t!("Steam Families: share your library and set limits for children.")));
        if !self.asked[3] {
            self.asked[3] = true;
            ctx.send(Command::LoadFamily { language: language() });
        }
        let status = ctx.state.family.clone();
        if not_ready(ui, t, status.as_ref()) {
            self.asked[3] = false;
        }
        let Some(Ok(status)) = status else { return };
        let Some(family) = &status.family else {
            w::panel(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(w::text_semibold(t!("You're not in a Steam Family"), 16.0, t.text));
                ui.add_space(4.0);
                wrapped(ui, w::text(t!("A family shares games between up to six accounts, with limits for children. Creating or joining one happens on Steam's site."), 13.5, t.text_dim));
                if status.cooldown > 0 {
                    ui.add_space(6.0);
                    ui.label(w::text(t!("You can join a family again in {days} days.", days = status.cooldown.div_ceil(86_400)), 13.0, t.away));
                }
                for (_, inviter) in &status.invites {
                    let p = ctx.state.persona(*inviter);
                    ui.add_space(6.0);
                    ui.label(w::text(t!("{name} invited you to their family.", name = p.display_name()), 13.5, t.accent));
                }
                ui.add_space(10.0);
                if w::button(ui, t, Some(icons::USERS_THREE), t!("Create or join a family"), w::Kind::Primary, w::Size::Md).clicked() {
                    web("https://store.steampowered.com/account/familymanagement");
                }
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.label(w::text_bold(&family.name, 22.0, t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if w::button(ui, t, Some(icons::GEAR), t!("Manage family"), w::Kind::Secondary, w::Size::Sm).on_hover_text(t!("Invites, removing members and limits open on Steam's site")).clicked() {
                    web("https://store.steampowered.com/account/familymanagement");
                }
            });
        });
        let mut facts = vec![t!("{n} members", n = family.members.len()).to_string(), t!("{n} free spots", n = family.free_spots).to_string()];
        if !family.country.is_empty() {
            facts.push(family.country.clone());
        }
        ui.label(w::text(facts.join("  ·  "), 13.0, t.text_dim));
        if family.slot_cooldown > 0 {
            ui.label(w::text(t!("A new member can join in {days} days.", days = family.slot_cooldown.div_ceil(86_400)), 12.5, t.away));
        }

        section(ui, t, t!("Members"));
        let me = ctx.state.steam_id;
        for m in &family.members {
            let p = ctx.state.persona(m.steam_id);
            w::panel(t).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 40.0, Some(crate::shell::status_color(t, &p)));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let name = if p.display_name().is_empty() { m.steam_id.to_string() } else { p.display_name() };
                            ui.label(w::text_semibold(name, 14.5, t.text));
                            if m.steam_id == me {
                                ui.label(w::text(t!("(you)"), 12.5, t.text_faint));
                            }
                            let (label, color) = match m.role {
                                FamilyRole::Adult => (t!("Adult"), t.accent),
                                FamilyRole::Child => (t!("Child"), t.away),
                                FamilyRole::Unknown => (t!("Member"), t.text_dim),
                            };
                            w::pill(ui, label, color, color.gamma_multiply(0.15));
                        });
                        let mut line = t!("Joined {date}", date = date(Some(m.joined).filter(|j| *j > 0))).to_string();
                        if m.cooldown > 0 {
                            line.push_str(&format!("  ·  {}", t!("can join another family in {days} days", days = m.cooldown.div_ceil(86_400))));
                        }
                        ui.label(w::text(line, 12.5, t.text_dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if w::button(ui, t, None, t!("Profile"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            ctx.navigate("profile", Some(m.steam_id.to_string()));
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        if !family.invites.is_empty() {
            section(ui, t, t!("Invited"));
            for (id, _) in &family.invites {
                let p = ctx.state.persona(*id);
                info_row(ui, t, &p.display_name(), t!("Hasn't accepted yet"), None);
            }
        }

        section(ui, t, t!("Family library"));
        let shareable = family.shared.iter().filter(|a| !a.excluded).count();
        ui.horizontal(|ui| {
            w::search(ui, t, &mut self.shared_filter, t!("Search the family library"), 320.0);
            ui.add_space(12.0);
            ui.label(w::text(t!("{n} games can be shared", n = shareable), 13.0, t.text_dim));
        });
        ui.add_space(8.0);
        let needle = self.shared_filter.to_lowercase();
        for a in family.shared.iter().filter(|a| needle.is_empty() || a.name.to_lowercase().contains(&needle)) {
            let resp = egui::Frame::new().inner_margin(Margin::symmetric(6, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(92.0, 43.0), Sense::hover());
                    w::image_or_skeleton(ui, t, r, &art::header(a.app_id), t.corner_small());
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(w::text_medium(&a.name, 13.5, if a.excluded { t.text_faint } else { t.text }));
                        let owners: Vec<String> = a.owners.iter().map(|o| if *o == me { t!("you").to_string() } else { ctx.state.persona(*o).display_name() }).collect();
                        let mut line = t!("Owned by {names}", names = owners.join(", ")).to_string();
                        if a.excluded {
                            line.push_str(&format!("  ·  {}", t!("can't be shared")));
                        }
                        ui.label(w::text(line, 12.0, t.text_dim));
                    });
                });
            });
            if resp.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                ctx.navigate("store", Some(a.app_id.to_string()));
            }
        }
    }
}

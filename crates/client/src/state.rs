//! A snapshot of the session built purely from [`Event`]s.

use crate::{Event, Guard};
use model::{AppId, ChatMessage, OwnedApp, Persona, ProfileInfo, Relationship, SteamId};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    LoggedOut,
    Connecting,
    /// Waiting for a QR code scan.
    Qr(String),
    Guard(Vec<Guard>),
    LoggedIn,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub phase: Phase,
    pub steam_id: SteamId,
    pub account: String,
    pub persona_name: String,
    pub country: String,
    pub level: Option<u32>,
    /// Wallet balance (cents) and currency code.
    pub wallet: Option<(i64, i32)>,
    /// Wallet money not released yet (recent Market sales, refunds), in cents.
    pub wallet_pending: i64,
    pub profiles: HashMap<SteamId, ProfileInfo>,
    /// Loaded inventories by (account, app, context).
    pub inventories: HashMap<(SteamId, AppId, u64), model::Inventory>,
    /// Market prices by (app, market hash name).
    pub market_prices: HashMap<(AppId, String), Result<model::MarketPrice, String>>,
    pub my_listings: Option<Result<model::MyListings, String>>,
    /// Names of games not in your library.
    pub app_names: HashMap<AppId, String>,
    pub email: Option<(String, bool)>,
    /// Last Steam Cloud result per game, and games syncing right now.
    pub cloud: HashMap<AppId, model::CloudReport>,
    pub cloud_syncing: std::collections::HashSet<AppId>,
    pub comments: HashMap<SteamId, Result<model::Comments, String>>,
    pub badges: HashMap<SteamId, Result<model::Badges, String>>,
    pub notifications: Option<model::Notifications>,
    pub support_tickets: Option<Result<Vec<model::SupportTicket>, String>>,
    pub account_security: Option<Result<model::AccountSecurity, String>>,
    /// Purchase history loaded so far (pages appended).
    pub purchases: Option<Result<model::PurchasePage, String>>,
    pub licenses: Option<Result<Vec<model::License>, String>>,
    pub family: Option<Result<model::FamilyStatus, String>>,
    pub support_messages: HashMap<String, Result<Vec<model::SupportMessage>, String>>,
    /// Market fee rules for your wallet currency (USD-style until Steam says).
    pub fee_rules: model::fees::Rules,
    /// Games with inventories, by account.
    pub inventory_apps: HashMap<SteamId, Result<Vec<model::InventoryApp>, String>>,
    /// Games on profiles (yours and friends'), or why they're hidden.
    pub player_games: HashMap<SteamId, Result<Vec<model::PlayerGame>, String>>,
    /// Your achievements per game: (unlocked, total).
    pub achievements: HashMap<AppId, (u32, u32)>,
    /// Game server details asked for with "View server info".
    pub servers: HashMap<String, Result<crate::a2s::ServerInfo, String>>,
    /// A game's achievement list, once loaded.
    pub achievement_list: HashMap<AppId, Result<Vec<model::Achievement>, String>>,
    pub downloads: HashMap<AppId, model::DownloadStatus>,
    pub download_queue: Vec<AppId>,
    pub wishlist: Option<Vec<(AppId, u64)>>,
    pub cart: Option<Vec<model::CartLine>>,
    pub license_count: usize,
    /// `None` until PICS has answered.
    pub apps: Option<BTreeMap<AppId, OwnedApp>>,
    pub friends: HashMap<SteamId, Relationship>,
    pub personas: HashMap<SteamId, Persona>,
    /// Conversations by friend, oldest message first.
    pub chats: HashMap<SteamId, Vec<ChatMessage>>,
    /// Unread message counts by friend.
    pub unread: HashMap<SteamId, usize>,
    /// When each friend last sent a typing notice (unix seconds).
    pub typing: HashMap<SteamId, u64>,
    pub playing: Option<AppId>,
    /// The running game's lobby, if it's in one ("Invite to lobby").
    pub game_lobby: Option<u64>,
    /// The running game's join string, rich presence `connect` ("Invite to game").
    pub game_connect: Option<String>,
    /// How each game was last started (SteamInRust / official Steam / no Steam).
    pub launch_modes: HashMap<AppId, crate::LaunchMode>,
    pub last_error: Option<String>,
    pub guard_error: Option<String>,
    /// Community pages loaded so far (Workshop items, guides, discussions).
    pub hub: crate::hub::Cache,
    /// Online, offline mode, or reconnecting.
    pub link: crate::Link,
    /// Play time recorded while not connected: game -> (seconds in all,
    /// seconds not on Steam yet).
    pub offline_playtime: HashMap<AppId, (u64, u64)>,
    /// Games whose Steam Cloud upload waits for a connection.
    pub cloud_pending: std::collections::HashSet<AppId>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl State {
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::Connecting => {
                self.phase = Phase::Connecting;
                self.last_error = None;
            }
            Event::QrChallenge { url } => self.phase = Phase::Qr(url.clone()),
            Event::GuardRequired { guards } => {
                self.phase = Phase::Guard(guards.clone());
                self.guard_error = None;
            }
            Event::GuardCodeRejected(msg) => self.guard_error = Some(msg.clone()),
            Event::LoginFailed(msg) => {
                *self = State { last_error: Some(msg.clone()), ..Default::default() };
            }
            Event::LoggedIn { steam_id, account } => {
                self.phase = Phase::LoggedIn;
                self.steam_id = *steam_id;
                self.account = account.clone();
            }
            Event::AccountInfo { persona_name, country } => {
                self.persona_name = persona_name.clone();
                self.country = country.clone();
            }
            Event::Level(l) => self.level = Some(*l),
            Event::Wallet { balance, currency, pending } => {
                self.wallet = Some((*balance, *currency));
                self.wallet_pending = *pending;
            }
            Event::Profile(p) => {
                self.profiles.insert(p.steam_id, p.clone());
            }
            Event::Inventory(inv) => {
                self.inventories.insert((inv.steam_id, inv.app_id, inv.context_id), inv.clone());
            }
            Event::MarketPrice { app_id, hash_name, result } => {
                // Keep order-book numbers from an earlier full load.
                let mut result = result.clone();
                if let (Ok(new), Some(Ok(old))) = (&mut result, self.market_prices.get(&(*app_id, hash_name.clone()))) {
                    new.lowest_sell = new.lowest_sell.or(old.lowest_sell);
                    new.highest_buy = new.highest_buy.or(old.highest_buy);
                    new.sell_count = new.sell_count.or(old.sell_count);
                    new.buy_count = new.buy_count.or(old.buy_count);
                }
                if let Ok(Some(r)) = result.as_ref().map(|p| p.fee_rules) {
                    self.fee_rules = r;
                }
                self.market_prices.insert((*app_id, hash_name.clone()), result);
            }
            Event::MyListings(r) => self.my_listings = Some(r.clone()),
            Event::Comments { steam_id, result } => {
                self.comments.insert(*steam_id, result.clone());
            }
            Event::Badges { steam_id, result } => {
                self.badges.insert(*steam_id, result.clone());
            }
            Event::CommentPosted { .. } => {}
            Event::ServerInfo { addr, result } => {
                self.servers.insert(addr.clone(), result.clone());
            }
            Event::Blocked { steam_id, result: Ok(true) } => {
                self.friends.remove(steam_id);
            }
            Event::Blocked { .. } => {}
            Event::WorkshopTags { .. } => {}
            Event::AppNames(list) => self.app_names.extend(list.iter().cloned()),
            Event::Email { address, validated } => self.email = Some((address.clone(), *validated)),
            Event::Regions(_) => {}
            Event::SteamCollections(_) => {}
            Event::CloudSyncing { app_id } => {
                self.cloud_syncing.insert(*app_id);
            }
            Event::CloudReport { report, .. } => {
                self.cloud_syncing.remove(&report.app_id);
                self.cloud.insert(report.app_id, report.clone());
            }
            Event::Notifications(n) => self.notifications = Some(n.clone()),
            Event::NotificationsArrived(_) => {}
            Event::SupportTickets(r) => self.support_tickets = Some(r.clone()),
            Event::AccountSecurity(r) => self.account_security = Some(r.clone()),
            Event::Purchases { more, result } => match (more, &mut self.purchases, result) {
                (true, Some(Ok(page)), Ok(next)) => {
                    page.rows.extend(next.rows.iter().cloned());
                    page.more = next.more.clone();
                }
                _ => self.purchases = Some(result.clone()),
            },
            Event::LicenseList(r) => self.licenses = Some(r.clone()),
            Event::Family(r) => self.family = Some(r.clone()),
            Event::SupportTicket { id, result } => {
                self.support_messages.insert(id.clone(), result.clone());
            }
            Event::FeeRules(r) => self.fee_rules = *r,
            Event::ListingCancelled { listing_id, result: Ok(()) } => {
                if let Some(Ok(l)) = &mut self.my_listings {
                    l.listings.retain(|x| x.listing_id != *listing_id);
                }
            }
            Event::BuyOrderCancelled { order_id, result: Ok(()) } => {
                if let Some(Ok(l)) = &mut self.my_listings {
                    l.buy_orders.retain(|x| x.order_id != *order_id);
                }
            }
            Event::ItemListed { asset_id, result: Ok(_) } => {
                // The item leaves the inventory once listed.
                for inv in self.inventories.values_mut() {
                    inv.items.retain(|i| i.asset_id != *asset_id);
                }
            }
            Event::ItemListed { .. } | Event::ListingCancelled { .. } | Event::BuyOrderCancelled { .. } => {}
            Event::InventoryApps { steam_id, result } => {
                self.inventory_apps.insert(*steam_id, result.clone());
            }
            Event::PlayerGames { steam_id, result } => {
                self.player_games.insert(*steam_id, result.clone());
            }
            Event::Achievements { app_id, result } => {
                if let Ok(list) = result {
                    let unlocked = list.iter().filter(|a| a.unlocked).count() as u32;
                    self.achievements.insert(*app_id, (unlocked, list.len() as u32));
                }
                self.achievement_list.insert(*app_id, result.clone());
            }
            Event::AchievementProgress(list) => {
                for (app, unlocked, total) in list {
                    self.achievements.insert(*app, (*unlocked, *total));
                }
            }
            Event::Licenses { count } => self.license_count = *count,
            Event::Apps(apps) => {
                for a in apps.iter() {
                    for path in &a.art {
                        model::art::remember(a.app_id, path);
                    }
                }
                self.apps = Some(apps.iter().map(|a| (a.app_id, a.clone())).collect());
            }
            Event::Friends(list) => self.friends = list.iter().copied().collect(),
            Event::Personas(list) => {
                for p in list {
                    self.personas.insert(p.steam_id, p.clone());
                }
            }
            Event::Message(m) => {
                let chat = self.chats.entry(m.friend).or_default();
                // The server echoes our own messages; skip exact duplicates.
                if !chat.iter().rev().take(5).any(|c| c.from == m.from && c.text == m.text && c.invite == m.invite && c.time.abs_diff(m.time) < 5) {
                    chat.push(m.clone());
                }
                if m.from != self.steam_id {
                    *self.unread.entry(m.friend).or_default() += 1;
                    self.typing.remove(&m.friend);
                }
            }
            Event::Typing { from } => {
                self.typing.insert(*from, now());
            }
            Event::History { friend, messages } => {
                let chat = self.chats.entry(*friend).or_default();
                let newest_loaded = messages.last().map(|m| m.time).unwrap_or(0);
                let mut merged = messages.clone();
                merged.extend(chat.iter().filter(|m| m.time > newest_loaded).cloned());
                *chat = merged;
            }
            Event::GameStarted { app_id } => {
                self.playing = Some(*app_id);
                self.game_lobby = None;
                self.game_connect = None;
                // Show it on our own status right away, like Steam.
                let me = self.steam_id;
                self.personas.entry(me).or_insert_with(|| Persona { steam_id: me, ..Default::default() }).game_app_id = Some(*app_id);
            }
            Event::GameStopped { .. } => {
                self.playing = None;
                self.game_lobby = None;
                self.game_connect = None;
                if let Some(p) = self.personas.get_mut(&self.steam_id) {
                    p.game_app_id = None;
                    p.game_name = None;
                }
            }
            Event::GameLobby { app_id, lobby } if self.playing == Some(*app_id) => self.game_lobby = *lobby,
            Event::GameConnect { app_id, connect } if self.playing == Some(*app_id) => self.game_connect = connect.clone(),
            Event::GameLobby { .. } | Event::GameConnect { .. } => {}
            Event::LoggedOut { reason } => {
                *self = State { last_error: Some(reason.clone()), ..Default::default() };
            }
            Event::Error(msg) => self.last_error = Some(msg.clone()),
            Event::KeyActivated(_) | Event::WorkshopUnsubscribed { .. } | Event::StoreError(_) => {}
            Event::WorkshopResults { .. } | Event::WorkshopSubscribed { .. } | Event::WorkshopSync { .. } => {}
            Event::Download(d) => {
                self.downloads.insert(d.app_id, d.clone());
            }
            Event::DownloadQueue(q) => self.download_queue = q.clone(),
            Event::DownloadFinished { app_id, result } => {
                if let Some(d) = self.downloads.get_mut(app_id) {
                    match result {
                        Ok(_) => d.phase = model::DownloadPhase::Done,
                        Err(e) => {
                            d.phase = model::DownloadPhase::Failed;
                            d.error = Some(e.clone());
                        }
                    }
                }
            }
            Event::Wishlist(w) => self.wishlist = Some(w.clone()),
            Event::Cart(c) => self.cart = Some(c.clone()),
            Event::Unhandled { .. } => {}
            Event::LaunchMode { app_id, mode } => {
                self.launch_modes.insert(*app_id, mode.clone());
            }
            Event::Hub(r) => self.hub.apply(r),
            Event::ConnectionError { message, .. } => {
                *self = State { last_error: Some(message.clone()), ..Default::default() };
            }
            Event::Link(l) => self.link = *l,
            Event::OfflinePlaytime(list) => self.offline_playtime = list.iter().map(|&(a, total, pending)| (a, (total, pending))).collect(),
            Event::CloudPending(list) => self.cloud_pending = list.iter().copied().collect(),
            Event::CloudSkipped { .. } => {}
        }
    }

    /// Connected to Steam (not offline mode, not reconnecting).
    pub fn online(&self) -> bool {
        self.link == crate::Link::Online
    }

    pub fn me(&self) -> Option<&Persona> {
        self.personas.get(&self.steam_id)
    }

    pub fn persona(&self, id: SteamId) -> Persona {
        self.personas.get(&id).cloned().unwrap_or_else(|| Persona { steam_id: id, ..Default::default() })
    }

    /// Accepted friends with their persona (if known yet).
    pub fn friend_list(&self) -> Vec<Persona> {
        let mut out: Vec<Persona> = self
            .friends
            .iter()
            .filter(|(_, r)| **r == Relationship::Friend)
            .map(|(id, _)| self.persona(*id))
            .collect();
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    pub fn pending_requests(&self) -> usize {
        self.friends.values().filter(|r| **r == Relationship::RequestRecipient).count()
    }

    pub fn total_unread(&self) -> usize {
        self.unread.values().sum()
    }

    /// True if the friend sent a typing notice in the last 8 seconds.
    pub fn is_typing(&self, id: SteamId) -> bool {
        self.typing.get(&id).is_some_and(|t| now().saturating_sub(*t) < 8)
    }

    pub fn app_name(&self, id: AppId) -> Option<&str> {
        self.apps.as_ref().and_then(|a| a.get(&id)).map(|a| a.name.as_str()).or_else(|| self.app_names.get(&id).map(|s| s.as_str()))
    }
}

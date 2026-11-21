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
    /// Your market history so far (newest first), how many rows Steam has,
    /// or why it couldn't load.
    pub market_history: Option<Result<(Vec<model::HistoryRow>, u32), String>>,
    pub trade_offers: Option<Result<model::TradeOffers, String>>,
    pub chat_groups: Option<Result<Vec<model::ChatGroup>, String>>,
    pub chat_group_members: HashMap<u64, Vec<SteamId>>,
    /// Messages per (group, room), oldest first, and whether older ones exist.
    pub group_messages: HashMap<(u64, u64), Vec<model::GroupMessage>>,
    pub group_more: HashMap<(u64, u64), bool>,
    pub profile_edit: Option<Result<model::ProfileEdit, String>>,
    /// Badge pages by their link.
    pub badge_pages: HashMap<String, Result<model::BadgeDetail, String>>,
    /// Location lists by (country, state).
    pub locations: HashMap<(Option<String>, Option<String>), Vec<(String, String)>>,
    /// Names of games not in your library.
    pub app_names: HashMap<AppId, String>,
    pub email: Option<(String, bool)>,
    /// Last Steam Cloud result per game, and games syncing right now.
    pub cloud: HashMap<AppId, model::CloudReport>,
    pub cloud_syncing: std::collections::HashSet<AppId>,
    /// Games whose Workshop items download before they start: (done, total, current title).
    pub workshop_preparing: HashMap<AppId, (u32, u32, String)>,
    pub comments: HashMap<SteamId, Result<model::Comments, String>>,
    pub badges: HashMap<SteamId, Result<model::Badges, String>>,
    pub notifications: Option<model::Notifications>,
    /// Your chat emoticons, stickers and effects (asked for once).
    pub emoticons: Option<model::Emoticons>,
    /// Your trade offer link.
    pub trade_link: Option<Result<String, String>>,
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
    /// Queued games that were set aside part-way, with how far they got.
    pub set_aside: HashMap<AppId, f32>,
    /// The store's discovery queue, once loaded.
    pub discovery: Option<Result<Vec<AppId>, String>>,
    /// Games running their first-time setup: (step, of, what).
    pub first_time_setup: HashMap<AppId, (usize, usize, String)>,
    /// Friends' reviews per game, and reviews by other people (to vote on).
    pub friend_reviews: HashMap<AppId, Result<Vec<(SteamId, model::MyReview)>, String>>,
    pub reviews_by: HashMap<(AppId, SteamId), model::MyReview>,
    /// Your votes on other people's reviews this session.
    pub review_votes: HashMap<u64, model::ReviewVote>,
    /// Someone's friends, reviews and groups, per profile.
    pub profile_friends: HashMap<SteamId, Result<Vec<SteamId>, String>>,
    pub profile_reviews: HashMap<SteamId, Result<Vec<model::ProfileReview>, String>>,
    pub profile_groups: HashMap<SteamId, Result<Vec<model::ProfileGroup>, String>>,
    /// The latest Community Market search: (query, game, start) and its answer.
    pub market_search: Option<((String, Option<AppId>, u32), Result<(Vec<model::MarketResult>, u32), String>)>,
    /// Your reviews, per game, once loaded (`None`: not written).
    pub my_reviews: HashMap<AppId, Result<Option<model::MyReview>, String>>,
    /// Games marked "Not interested" this session.
    pub ignored: std::collections::HashSet<AppId>,
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
    /// Group chats' roles, invites, bans and your settings for them (the
    /// group settings and notifications dialogs).
    pub chat_admin: HashMap<u64, Result<model::chat_admin::GroupAdmin, String>>,
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
                    new.median = new.median.take().or_else(|| old.median.clone());
                    new.volume = new.volume.or(old.volume);
                }
                if let Ok(Some(r)) = result.as_ref().map(|p| p.fee_rules) {
                    self.fee_rules = r;
                }
                self.market_prices.insert((*app_id, hash_name.clone()), result);
            }
            Event::MyListings(r) => self.my_listings = Some(r.clone()),
            Event::MarketHistory { start, result } => match (result, &mut self.market_history) {
                // A later page adds to what's there.
                (Ok((rows, total)), Some(Ok((have, t)))) if *start > 0 => {
                    for r in rows {
                        if !have.iter().any(|h| h.id == r.id) {
                            have.push(r.clone());
                        }
                    }
                    *t = *total;
                }
                (Err(_), Some(Ok(_))) if *start > 0 => {}
                _ => self.market_history = Some(result.clone()),
            },
            Event::TradeOffers(r) => self.trade_offers = Some(r.clone()),
            Event::ChatGroups(r) => self.chat_groups = Some(r.clone()),
            Event::ChatGroupMembers(list) => {
                for (g, who) in list {
                    self.chat_group_members.insert(*g, who.clone());
                }
            }
            Event::GroupHistory { group, room, result: Ok((msgs, more)), older } => {
                let key = (*group, *room);
                let list = self.group_messages.entry(key).or_default();
                // Merge: a page can overlap what live messages already added.
                for m in msgs {
                    if !list.iter().any(|x| x.time == m.time && x.ordinal == m.ordinal && x.sender == m.sender) {
                        list.push(m.clone());
                    }
                }
                list.sort_by_key(|m| (m.time, m.ordinal));
                if *older || !self.group_more.contains_key(&key) {
                    self.group_more.insert(key, *more);
                }
            }
            Event::GroupMessage(m) => {
                let list = self.group_messages.entry((m.group, m.room)).or_default();
                if !list.iter().any(|x| x.time == m.time && x.ordinal == m.ordinal && x.sender == m.sender) {
                    list.push(m.clone());
                    list.sort_by_key(|m| (m.time, m.ordinal));
                }
                // The room shows it as its latest message, unread unless yours.
                let mine = m.sender == self.steam_id;
                if let Some(Ok(groups)) = &mut self.chat_groups {
                    if let Some(g) = groups.iter_mut().find(|g| g.id == m.group) {
                        if let Some(r) = g.rooms.iter_mut().find(|r| r.id == m.room) {
                            r.last_message = m.text.clone();
                            r.last_time = m.time;
                            r.last_sender = m.sender;
                            r.unread = r.unread || !mine;
                        }
                        g.unread = !g.muted && g.rooms.iter().any(|r| r.unread);
                    }
                }
            }
            Event::GroupRoomRead { group, room } => {
                if let Some(Ok(groups)) = &mut self.chat_groups {
                    if let Some(g) = groups.iter_mut().find(|g| g.id == *group) {
                        if let Some(r) = g.rooms.iter_mut().find(|r| r.id == *room) {
                            r.unread = false;
                        }
                        g.unread = !g.muted && g.rooms.iter().any(|r| r.unread);
                    }
                }
            }
            Event::GroupHistory { .. } | Event::GroupMessageFailed { .. } => {}
            Event::ProfileEdit(r) => self.profile_edit = Some(r.clone()),
            Event::Badge { url, result } => {
                self.badge_pages.insert(url.clone(), result.clone());
            }
            Event::BadgeCrafted { .. } => {}
            Event::Locations { country, state, result: Ok(l) } => {
                self.locations.insert((country.clone(), state.clone()), l.clone());
            }
            Event::Locations { .. } | Event::ProfileSaved { .. } => {}
            // The store keeps these.
            Event::PointsBalance(_) | Event::RewardEligible(_) | Event::RewardInventory { .. } | Event::PointsRedeemed { .. } => {}
            // Shown right away; the list is loaded again afterwards.
            Event::TradeOfferAnswered { id, action, result: Ok(confirm) } => {
                if let Some(Ok(o)) = &mut self.trade_offers {
                    for x in o.received.iter_mut().chain(o.sent.iter_mut()).filter(|x| x.id == *id) {
                        x.state = match action {
                            _ if *confirm => model::TradeState::NeedsConfirmation,
                            model::TradeAction::Accept => model::TradeState::Accepted,
                            model::TradeAction::Decline => model::TradeState::Declined,
                            model::TradeAction::Cancel => model::TradeState::Cancelled,
                        };
                    }
                }
            }
            Event::TradeOfferAnswered { .. } | Event::TradeOfferSent { .. } | Event::MarketPricesDone { .. } => {}
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
            Event::SteamCollections(_) | Event::SteamCollectionsSaved(_) | Event::QuickInvite(_) | Event::QuickInviteRedeemed { .. } | Event::UserSearch(_) => {}
            Event::CloudSyncing { app_id } => {
                self.workshop_preparing.remove(app_id);
                self.cloud_syncing.insert(*app_id);
            }
            Event::WorkshopPreparing { app_id, done, total, title } => {
                if *total == 0 {
                    self.workshop_preparing.remove(app_id);
                } else {
                    self.workshop_preparing.insert(*app_id, (*done, *total, title.clone()));
                }
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
                let same = |c: &model::ChatMessage| {
                    (c.ordinal == m.ordinal && c.time == m.time && c.time != 0) || (c.from == m.from && c.text == m.text && c.raw == m.raw && c.invite == m.invite && c.time.abs_diff(m.time) < 5)
                };
                if !chat.iter().rev().take(5).any(same) {
                    chat.push(m.clone());
                }
                if m.from != self.steam_id {
                    *self.unread.entry(m.friend).or_default() += 1;
                    self.typing.remove(&m.friend);
                }
            }
            Event::MessageReaction { to, time, ordinal, reactor, reaction, sticker, add } => {
                let me = *reactor == self.steam_id;
                match to {
                    crate::ChatTarget::Friend(f) => {
                        if let Some(m) = self.chats.get_mut(f).and_then(|c| c.iter_mut().rev().find(|m| m.time == *time as u64 && m.ordinal == *ordinal)) {
                            model::Reaction::apply(&mut m.reactions, reaction, *sticker, *add, me);
                        }
                    }
                    crate::ChatTarget::Group { group, room } => {
                        if let Some(m) = self.group_messages.get_mut(&(*group, *room)).and_then(|c| c.iter_mut().rev().find(|m| m.time == *time && m.ordinal == *ordinal)) {
                            model::Reaction::apply(&mut m.reactions, reaction, *sticker, *add, me);
                        }
                    }
                }
            }
            Event::Emoticons(e) => self.emoticons = Some(e.clone()),
            // Told to the user by the app; nothing to keep.
            Event::ChatImageSent { .. } | Event::MessageReported(_) | Event::ChatGroupCreated(_) | Event::ChatGroupFailed(_) => {}
            Event::TradeLink(r) => self.trade_link = Some(r.clone()),
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
            // A developer attaching a game by hand: nothing to show.
            Event::GameAttached { .. } | Event::GameOverlay { .. } => {}
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
            Event::WorkshopResults { .. } | Event::WorkshopSubscribed { .. } | Event::WorkshopSync { .. } | Event::FreeLicense { .. } => {}
            Event::Download(d) => {
                self.set_aside.remove(&d.app_id);
                self.downloads.insert(d.app_id, d.clone());
            }
            Event::DownloadQueue(q) => self.download_queue = q.clone(),
            Event::VerifyFinished { app_id, .. } => {
                if self.downloads.get(app_id).is_some_and(|d| d.verifying()) {
                    self.downloads.remove(app_id);
                }
            }
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
            Event::DownloadStopped { app_id } => {
                // Back in the queue like any other queued game; only how far
                // it got is kept, for the queue's "45% done".
                if let Some(d) = self.downloads.remove(app_id) {
                    self.set_aside.insert(*app_id, d.fraction());
                }
            }
            Event::Wishlist(w) => self.wishlist = Some(w.clone()),
            Event::WishlistChanging { app_id, add } => {
                let list = self.wishlist.get_or_insert_with(Vec::new);
                list.retain(|(a, _)| a != app_id);
                if *add {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    list.push((*app_id, now));
                }
            }
            Event::DiscoveryQueue(q) => self.discovery = Some(q.clone()),
            Event::FriendReviews { app_id, result } => {
                self.friend_reviews.insert(*app_id, result.clone());
            }
            Event::ReviewsBy { app_id, reviews } => {
                for (who, r) in reviews {
                    self.reviews_by.insert((*app_id, *who), r.clone());
                }
            }
            Event::ReviewVoted { review, vote, result } => {
                if result.is_ok() {
                    self.review_votes.insert(*review, *vote);
                }
            }
            Event::ProfileFriends { steam_id, result } => {
                self.profile_friends.insert(*steam_id, result.clone());
            }
            Event::ProfileReviews { steam_id, result } => {
                self.profile_reviews.insert(*steam_id, result.clone());
            }
            Event::ProfileGroups { steam_id, result } => {
                self.profile_groups.insert(*steam_id, result.clone());
            }
            Event::MarketSearch { query, app, start, result } => self.market_search = Some(((query.clone(), *app, *start), result.clone())),
            Event::MyReview { app_id, result } => {
                self.my_reviews.insert(*app_id, result.clone());
            }
            Event::ReviewSaved { .. } => {}
            Event::FirstTimeSetup { app_id, done, total, step } => {
                if *total == 0 {
                    self.first_time_setup.remove(app_id);
                } else {
                    self.first_time_setup.insert(*app_id, (*done, *total, step.clone()));
                }
            }
            Event::AppIgnored { app_id, ignore, result } => {
                if result.is_ok() {
                    if *ignore {
                        self.ignored.insert(*app_id);
                    } else {
                        self.ignored.remove(app_id);
                    }
                }
            }
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
            Event::ChatAdmin(e) => crate::chat_admin::apply(self, e),
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

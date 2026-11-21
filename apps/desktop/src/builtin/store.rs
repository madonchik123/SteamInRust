//! Store: rendered natively from Steam's store JSON (no web view).
//! Payment is the only step that leaves the app: Steam only accepts it on its website.

use ui::widgets::EditMenu;
use i18n::t;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use model::AppId;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use store::browse::BrowseInfo;
use store::{Details, Item, Section, StoreClient, html::Block};
use ui::{Theme, icons, widgets as w};

#[path = "store_more.rs"]
mod more;
#[path = "store_discovery.rs"]
mod discovery;
#[path = "store_review.rs"]
mod review;
#[path = "store_micro.rs"]
mod micro;
#[path = "store_reviews_list.rs"]
mod reviews_list;
#[path = "store_specs.rs"]
mod specs;
#[path = "store_home.rs"]
mod home;
#[path = "store_page.rs"]
mod store_page;
#[path = "store_points.rs"]
mod points;
#[path = "store_redeem.rs"]
mod redeem;
#[path = "store_offers.rs"]
mod offers;
#[path = "store_curators.rs"]
mod curators;
#[path = "store_suggest.rs"]
pub(crate) mod suggest;
#[path = "store_demo.rs"]
pub mod demo;

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}
type Shared<T> = Arc<Mutex<Load<T>>>;

struct Players {
    asked: std::time::Instant,
    slot: Shared<Option<u32>>,
    last: Option<u32>,
}

#[derive(Clone, PartialEq)]
enum View {
    Home,
    Section(String),
    Search(String),
    /// Everything by a developer or publisher: ("developer" | "publisher", name).
    Creator(String, String),
    App(AppId),
    Browse,
    Wishlist,
    Cart,
    /// The discovery queue, at this game (one past the end: done).
    Discovery(usize),
    /// The discovery feed: the whole queue as a list.
    Discover,
    /// The Points Shop.
    Points,
    /// A bundle's or package's page.
    Offer(store::offers::OfferKind, u32),
    /// The curators you follow, and the most followed.
    Curators,
    /// One curator's page.
    Curator(u32),
}

pub struct Store {
    /// A view opened inside the store, to record in Back/Forward history.
    nav_out: Option<Option<String>>,
    /// Set while the app navigates us (Back, Forward, links): don't record again.
    in_nav: bool,
    /// A free game being added to the account.
    free_pending: Option<AppId>,
    /// Text in the Browse page's tag search box.
    tag_query: String,
    rt: tokio::runtime::Handle,
    egui: egui::Context,
    api: StoreClient,
    front: Option<Shared<(Vec<Item>, Vec<Section>)>>,
    details: HashMap<AppId, Shared<Details>>,
    browse: HashMap<AppId, Shared<BrowseInfo>>,
    /// People playing each game right now, refreshed every minute.
    players: HashMap<AppId, Players>,
    /// Up and down votes of the last 30 days, per game.
    recent: HashMap<AppId, Shared<(u64, u64)>>,
    tags: Option<Shared<HashMap<u32, String>>>,
    /// When the tag names were last asked for (to ask again if that failed).
    tags_asked: Option<std::time::Instant>,
    /// (country, language) the caches were loaded with.
    locale: (String, String),
    /// Content descriptors per app, for the mature content filter.
    ratings: Arc<Mutex<HashMap<AppId, Vec<u32>>>>,
    ratings_pending: Vec<AppId>,
    ratings_busy: Arc<Mutex<bool>>,
    /// Descriptor ids the user chose to hide.
    hidden: std::collections::BTreeSet<u32>,
    /// Mature pages the user chose to view this session.
    allowed: std::collections::HashSet<AppId>,
    /// Open hidden games' pages without the warning ("Don't ask again").
    never_ask: bool,
    search: Option<(String, Shared<Vec<Item>>)>,
    view: View,
    hero: usize,
    hero_changed: f64,
    shelf_page: HashMap<String, usize>,
    shot: usize,
    /// The strip item last scrolled into view, so a new pick gets centered once.
    shot_centered: Option<usize>,
    /// Games on your account (library + installed), for "In library" marks.
    owned: std::collections::HashSet<AppId>,
    /// The trailer playing in the media area: (source URL, player).
    trailer: Option<(String, video::Player)>,
    /// Games whose live broadcasts were already asked for.
    pub(crate) live_asked: std::collections::HashSet<AppId>,
    /// Frames so far (counted in `tick`), and the last one the store was
    /// drawn in: a trailer or hover clip left playing on a page you've left
    /// is freed.
    frames: u64,
    drawn_frame: u64,
    filters: store::query::Filters,
    browse_page: u32,
    query: Option<(store::query::Filters, u32, Shared<(Vec<store::query::QueryItem>, u64)>)>,
    hide_owned: bool,
    wish_items: Option<(Vec<AppId>, Shared<Vec<store::query::QueryItem>>)>,
    cart_names: Option<(Vec<u32>, Shared<Vec<(u32, String, String, AppId)>>)>,
    /// Steam's "Added to your cart!" popup, for one game.
    cart_popup: Option<more::CartPopup>,
    gift: Option<more::Gift>,
    history: HashMap<AppId, Shared<store::itad::PriceHistory>>,
    itad_key: String,
    price_log: std::collections::BTreeMap<AppId, Vec<(u64, u64)>>,
    price_log_dirty: bool,
    lists_requested: bool,
    /// Big Picture's search, kept apart from the store page's own.
    tv_search: Option<(String, Shared<Vec<Item>>)>,
    /// A discovery queue was asked for and hasn't arrived yet.
    discovery_wait: bool,
    /// When the queue was asked for (to stop waiting if Steam never answers).
    discovery_asked: Option<std::time::Instant>,
    /// Steam didn't send the queue in time: say so until asked again.
    discovery_late: bool,
    /// Writing your review of a game you own.
    review: review::ReviewBox,
    /// The clip playing on the card under the pointer.
    micro: micro::Micro,
    /// What players wrote, on game pages.
    reviews_list: reviews_list::ReviewList,
    /// Requirements tab and languages table.
    specs: specs::Specs,
    /// Text in the page bar's search.
    bar_query: String,
    /// Games suggested while typing in a search box.
    suggest: suggest::Suggest,
    /// The Points Shop's list and loaded pages.
    points: points::Points,
    /// The wishlist's sorting and filters.
    wish: more::WishView,
    /// Bundle and package pages.
    offers: offers::Offers,
    /// Curators, and what Steam says you follow and own.
    curators: curators::Curators,
    /// Names of the bundles in the cart.
    cart_bundles: Option<(Vec<u32>, Shared<Vec<(u32, String, String, AppId)>>)>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct Saved {
    hidden: Vec<u32>,
    never_ask: bool,
    itad_key: String,
    price_log: std::collections::BTreeMap<AppId, Vec<(u64, u64)>>,
}

impl Store {
    pub fn new(rt: tokio::runtime::Handle, egui: egui::Context) -> Self {
        Self {
            rt,
            egui,
            api: StoreClient::default(),
            front: None,
            details: HashMap::new(),
            browse: HashMap::new(),
            players: HashMap::new(),
            recent: HashMap::new(),
            tags: None,
            tags_asked: None,
            locale: ("us".into(), "english".into()),
            ratings: Default::default(),
            ratings_pending: Vec::new(),
            ratings_busy: Default::default(),
            hidden: [3, 4].into_iter().collect(),
            allowed: Default::default(),
            never_ask: false,
            filters: Default::default(),
            browse_page: 0,
            query: None,
            hide_owned: false,
            wish_items: None,
            cart_names: None,
            cart_popup: None,
            gift: None,
            history: HashMap::new(),
            nav_out: None,
            in_nav: false,
            free_pending: None,
            tag_query: String::new(),
            itad_key: String::new(),
            price_log: Default::default(),
            price_log_dirty: false,
            lists_requested: false,
            tv_search: None,
            discovery_wait: false,
            discovery_asked: None,
            discovery_late: false,
            review: Default::default(),
            micro: Default::default(),
            reviews_list: Default::default(),
            specs: Default::default(),
            bar_query: String::new(),
            suggest: Default::default(),
            points: Default::default(),
            wish: Default::default(),
            offers: Default::default(),
            curators: Default::default(),
            cart_bundles: None,
            search: None,
            view: View::Home,
            hero: 0,
            hero_changed: 0.0,
            shelf_page: HashMap::new(),
            shot: 0,
            shot_centered: None,
            owned: Default::default(),
            trailer: None,
            live_asked: Default::default(),
            frames: 0,
            drawn_frame: 0,
        }
    }

    fn fetch<T: Send + 'static>(&self, fut: impl Future<Output = store::Result<T>> + Send + 'static) -> Shared<T> {
        let slot = Arc::new(Mutex::new(Load::Loading));
        let (out, ctx) = (slot.clone(), self.egui.clone());
        self.rt.spawn(async move {
            let r = fut.await;
            *out.lock().unwrap() = match r {
                Ok(v) => Load::Ready(v),
                Err(e) => Load::Failed(e.to_string()),
            };
            ctx.request_repaint();
        });
        slot
    }

    fn details(&mut self, id: AppId) -> Option<Details> {
        if !self.details.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.details(id).await });
            self.details.insert(id, slot);
        }
        match &*self.details[&id].lock().unwrap() {
            Load::Ready(d) => Some(d.clone()),
            _ => None,
        }
    }

    fn browse_info(&mut self, id: AppId) -> Option<BrowseInfo> {
        if !self.browse.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.browse(&[id]).await.map(|mut v| v.pop().unwrap_or_default()) });
            self.browse.insert(id, slot);
        }
        match &*self.browse[&id].lock().unwrap() {
            Load::Ready(b) => Some(b.clone()),
            _ => None,
        }
    }

    /// People in the game right now (Steam's count). Asked again every
    /// minute while the page is open; the last number stays meanwhile.
    fn recent_votes(&mut self, id: AppId) -> Option<(u64, u64)> {
        if !self.recent.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.recent_votes(id).await });
            self.recent.insert(id, slot);
        }
        match &*self.recent[&id].lock().unwrap() {
            Load::Ready(v) => Some(*v),
            _ => None,
        }
    }

    fn current_players(&mut self, id: AppId) -> Option<u32> {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(60);
        if self.players.get(&id).is_none_or(|p| p.asked.elapsed() >= EVERY) {
            let last = self.players.get(&id).and_then(|p| p.last);
            let api = self.api.clone();
            let slot = self.fetch(async move { api.current_players(id).await });
            self.players.insert(id, Players { asked: std::time::Instant::now(), slot, last });
        }
        self.egui.request_repaint_after(EVERY);
        let p = self.players.get_mut(&id)?;
        let ready = match &*p.slot.lock().unwrap() {
            Load::Ready(n) => Some(*n),
            _ => None,
        };
        if let Some(n) = ready {
            p.last = n;
        }
        p.last
    }

    /// Every Steam tag whose name contains `query` (case-insensitive), by name.
    fn find_tags(&mut self, query: &str) -> Vec<(u32, String)> {
        self.tag_name(0);
        let q = query.trim().to_lowercase();
        let Some(slot) = &self.tags else { return Vec::new() };
        let Load::Ready(all) = &*slot.lock().unwrap() else { return Vec::new() };
        let mut found: Vec<(u32, String)> = all.iter().filter(|(_, n)| n.to_lowercase().contains(&q)).map(|(id, n)| (*id, n.clone())).collect();
        // Names that start with the query first, then alphabetical.
        found.sort_by_key(|(_, n)| (!n.to_lowercase().starts_with(&q), n.to_lowercase()));
        found
    }

    fn tag_name(&mut self, id: u32) -> Option<String> {
        self.tag_names_ready();
        match &*self.tags.as_ref()?.lock().unwrap() {
            Load::Ready(m) => m.get(&id).cloned(),
            _ => None,
        }
    }

    /// Steam's tag names, asked for once; asked again a little later when
    /// the answer failed or came back empty. True once they're here.
    fn tag_names_ready(&mut self) -> bool {
        const RETRY: std::time::Duration = std::time::Duration::from_secs(10);
        let state = self.tags.as_ref().map(|s| match &*s.lock().unwrap() {
            Load::Ready(m) if !m.is_empty() => Some(true),
            Load::Loading => None,
            _ => Some(false),
        });
        let retry = matches!(state, Some(Some(false))) && self.tags_asked.is_none_or(|a| a.elapsed() >= RETRY);
        if self.tags.is_none() || retry {
            let api = self.api.clone();
            self.tags = Some(self.fetch(async move { api.tag_names().await }));
            self.tags_asked = Some(std::time::Instant::now());
        } else if matches!(state, Some(Some(false))) {
            self.egui.request_repaint_after(RETRY);
        }
        matches!(state, Some(Some(true)))
    }

    /// Follow the wallet's country (prices) and the interface language.
    /// `false` while signed in but Steam hasn't said the country yet.
    fn sync_locale(&mut self, ctx: &Ctx) -> bool {
        let wallet = ctx.state.wallet.map(|w| w.1);
        let Some(country) = model::store_country(wallet, &ctx.state.country) else {
            if ctx.state.steam_id != 0 {
                return false;
            }
            return self.sync_locale_to("us".into());
        };
        self.sync_locale_to(country)
    }

    fn sync_locale_to(&mut self, country: String) -> bool {
        let language = i18n::language().steam_name().to_string();
        if self.locale != (country.clone(), language.clone()) {
            self.api = self.api.with_locale(&country, &language);
            self.locale = (country, language);
            self.front = None;
            self.details.clear();
            self.browse.clear();
            // Browse results carry prices too: load them again in the new currency.
            self.query = None;
            self.tags = None;
            self.ratings.lock().unwrap().clear();
            self.offers.clear();
            self.curators.clear();
            self.cart_bundles = None;
        }
        true
    }

    /// `Some(true)` = fine to show, `Some(false)` = filtered out by the
    /// user's content preferences, `None` = not known yet (show a placeholder).
    fn safe(&mut self, id: AppId) -> Option<bool> {
        if let Some(ids) = self.ratings.lock().unwrap().get(&id) {
            return Some(!ids.iter().any(|d| self.hidden.contains(d)));
        }
        if !self.ratings_pending.contains(&id) {
            self.ratings_pending.push(id);
        }
        None
    }

    /// Fetch content descriptors for everything that asked this frame.
    fn flush_ratings(&mut self) {
        if self.ratings_pending.is_empty() || *self.ratings_busy.lock().unwrap() {
            return;
        }
        *self.ratings_busy.lock().unwrap() = true;
        let ids: Vec<AppId> = self.ratings_pending.drain(..).take(100).collect();
        let (api, out, busy, ctx) = (self.api.clone(), self.ratings.clone(), self.ratings_busy.clone(), self.egui.clone());
        self.rt.spawn(async move {
            match api.browse(&ids).await {
                Ok(items) => {
                    let mut m = out.lock().unwrap();
                    for id in &ids {
                        m.insert(*id, Vec::new());
                    }
                    for i in items {
                        m.insert(i.app_id, i.content_descriptors);
                    }
                }
                Err(e) => tracing::warn!("content ratings: {e}"),
            }
            *busy.lock().unwrap() = false;
            ctx.request_repaint();
        });
    }

    fn open(&mut self, view: View) {
        self.trailer = None;
        if let View::App(_) | View::Discovery(_) = view {
            self.shot = 0;
            self.shot_centered = None;
        }
        // Opened from inside the store: put it in the app's Back history,
        // so Back returns to the previous store page.
        if !self.in_nav && view != self.view {
            self.nav_out = Some(view_arg(&view));
        }
        self.view = view;
    }

    /// Search the store for `q` and show the results.
    fn start_search(&mut self, q: String) {
        let api = self.api.clone();
        let qq = q.clone();
        let slot = self.fetch(async move { api.search(&qq).await });
        self.search = Some((q.clone(), slot));
        self.bar_query = q.clone();
        self.open(View::Search(q));
    }

    /// A developer's or publisher's games, like their store page.
    fn open_creator(&mut self, kind: &str, name: &str) {
        let (api, k, n) = (self.api.clone(), kind.to_string(), name.to_string());
        let slot = self.fetch(async move { api.creator(&k, &n).await });
        self.search = Some((format!("{kind}:{name}"), slot));
        self.open(View::Creator(kind.to_string(), name.to_string()));
    }

    /// Browse everything with one tag (clicked on a game page).
    pub(super) fn open_tag(&mut self, tag: u32) {
        self.filters = store::query::Filters { tags: vec![tag], ..Default::default() };
        self.browse_page = 0;
        self.open(View::Browse);
    }

    /// A tag id from its (localized) name, e.g. a genre on a game page.
    fn tag_by_name(&mut self, name: &str) -> Option<u32> {
        self.tag_name(0);
        match &*self.tags.as_ref()?.lock().unwrap() {
            Load::Ready(m) => m.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(id, _)| *id),
            _ => None,
        }
    }
}

/// What Big Picture reads from the store: the same data, cache and
/// content filter as the store page.
impl Store {
    /// The front page (featured games and sections) in the account's currency.
    pub(crate) fn front_for(&mut self, ctx: &Ctx) -> Option<Result<(Vec<Item>, Vec<Section>), String>> {
        if !self.sync_locale(ctx) {
            return None;
        }
        if self.front.is_none() {
            let api = self.api.clone();
            self.front = Some(self.fetch(async move { api.front().await }));
        }
        self.flush_ratings();
        self.front_data()
    }

    /// Search results for `query` (asked once per new query).
    pub(crate) fn search_for(&mut self, ctx: &Ctx, query: &str) -> Option<Result<Vec<Item>, String>> {
        if !self.sync_locale(ctx) {
            return None;
        }
        if self.tv_search.as_ref().is_none_or(|(q, _)| q != query) {
            let (api, q) = (self.api.clone(), query.to_string());
            let slot = self.fetch(async move { api.search(&q).await });
            self.tv_search = Some((query.to_string(), slot));
        }
        let (_, slot) = self.tv_search.as_ref()?;
        match &*slot.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(v) => Some(Ok(v.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        }
    }

    pub(crate) fn details_for(&mut self, ctx: &Ctx, id: AppId) -> Option<Details> {
        if !self.sync_locale(ctx) {
            return None;
        }
        self.details(id)
    }

    /// `Some(true)` = fine to show, `Some(false)` = hidden by the mature
    /// content settings, `None` = not known yet.
    pub(crate) fn shown(&mut self, id: AppId) -> Option<bool> {
        let r = self.safe(id);
        self.flush_ratings();
        r
    }
}

impl Extension for Store {
    fn manifest(&self) -> Manifest {
        Manifest { id: "store", name: "Store", author: "SteamInRust", description: "The Steam store, rendered natively", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("store", "Store", icons::STORE).main().order(10)]
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        self.wallet_event(event);
        // Steam's wishlist (after a move, or anything else) replaces the order shown.
        if let client::Event::Wishlist(_) = event {
            self.wish.pending = None;
        }
        if let client::Event::DiscoveryQueue(_) = event {
            self.discovery_wait = false;
            self.discovery_asked = None;
            self.discovery_late = false;
        }
        self.review.on_event(event, ctx);
        self.curators_event(event, ctx);
        if let client::Event::FreeLicense { app_id, result } = event {
            if self.free_pending == Some(*app_id) {
                self.free_pending = None;
            }
            match result {
                Ok(()) => ctx.toast(t!("Added to your library. You can install it from there.")),
                Err(e) => ctx.toast(i18n::tr_dyn(e).to_string()),
            }
        }
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        self.in_nav = true;
        self.nav_out = None;
        match arg {
            Some(a) if a.starts_with("section:") => self.open(View::Section(a.trim_start_matches("section:").to_string())),
            Some(a) if a.starts_with("search:") => self.start_search(a.trim_start_matches("search:").to_string()),
            Some(a) if a.starts_with("developer:") || a.starts_with("publisher:") => {
                let (kind, name) = a.split_once(':').unwrap_or_default();
                self.open_creator(kind, name);
            }
            Some(a) if a.starts_with("tag:") => {
                if let Ok(tag) = a.trim_start_matches("tag:").parse() {
                    self.open_tag(tag);
                }
            }
            Some("browse") => self.open(View::Browse),
            Some("wishlist") => self.open(View::Wishlist),
            Some("cart") => self.open(View::Cart),
            Some("discover") => self.open(View::Discover),
            Some("points") => self.open(View::Points),
            Some("curators") => self.open(View::Curators),
            Some(a) if a.starts_with("curator:") => {
                if let Ok(id) = a.trim_start_matches("curator:").parse() {
                    self.open(View::Curator(id));
                }
            }
            Some(a) if a.starts_with("bundle:") => {
                if let Ok(id) = a.trim_start_matches("bundle:").parse() {
                    self.open(View::Offer(store::offers::OfferKind::Bundle, id));
                }
            }
            Some(a) if a.starts_with("sub:") => {
                if let Ok(id) = a.trim_start_matches("sub:").parse() {
                    self.open(View::Offer(store::offers::OfferKind::Package, id));
                }
            }
            Some(a) if a.starts_with("discovery") => self.open(View::Discovery(a.trim_start_matches("discovery").trim_start_matches(':').parse().unwrap_or(0))),
            Some(a) => {
                if let Ok(id) = a.trim_start_matches("app:").parse() {
                    self.open(View::App(id));
                }
            }
            None => self.open(View::Home),
        }
        self.in_nav = false;
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        self.frames += 1;
        if self.frames.saturating_sub(self.drawn_frame) > 2 {
            self.trailer = None;
            self.micro = Default::default();
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        self.drawn_frame = self.frames;
        if let Some(arg) = self.nav_out.take() {
            ctx.navigate("store", arg);
        }
        let t = ctx.theme.clone();
        // Refresh the owned set only when the library changes size.
        let count = ctx.state.apps.as_ref().map(|a| a.len()).unwrap_or(0) + ctx.local.games.len();
        if count != self.owned.len() {
            self.owned = ctx.state.apps.as_ref().map(|a| a.keys().copied().collect()).unwrap_or_default();
            self.owned.extend(ctx.local.games.iter().map(|g| g.app_id));
        }
        if !self.sync_locale(ctx) {
            // Don't load prices in the wrong currency while sign-in finishes.
            let (r, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            w::skeleton(ui, &t, r.shrink(28.0), t.corner());
            return;
        }
        self.curators_tick(ctx);
        if !self.lists_requested && ctx.state.steam_id != 0 {
            self.lists_requested = true;
            ctx.send(client::Command::LoadWishlist);
            ctx.send(client::Command::LoadCart);
        }
        if self.front.is_none() {
            let api = self.api.clone();
            self.front = Some(self.fetch(async move { api.front().await }));
        }
        let bar = t.layout.page_bar;
        egui::ScrollArea::vertical().id_salt(("store", view_key(&self.view))).auto_shrink([false, false]).show(ui, |ui| {
            match self.view.clone() {
                View::App(id) if !self.allowed.contains(&id) && self.details_mature(id) => self.mature_gate(ui, &t, id),
                View::App(id) => {
                    if !bar {
                        ui.add_space(20.0);
                        w::centered(ui, store_page::MAX, 28.0, |ui| self.subnav(ui, &t, ctx));
                    }
                    self.app_page(ui, &t, ctx, id)
                }
                View::Discovery(pos) => self.discovery_page(ui, &t, ctx, pos),
                view => {
                    // The design's widths: the home page's 1388, the lists' 1560.
                    let max = if view == View::Home { 1388.0 } else { 1560.0 };
                    ui.add_space(t.dense(if bar { 44.0 } else { 20.0 }));
                    w::centered(ui, max, 28.0, |ui| {
                        if !bar {
                            self.subnav(ui, &t, ctx);
                            ui.add_space(18.0);
                        }
                        match view {
                            View::Home => self.home(ui, &t, ctx),
                            View::Section(id) => self.section(ui, &t, ctx, &id),
                            View::Search(q) => self.search_results(ui, &t, ctx, &t!("Results for “{q}”", q = q)),
                            View::Creator(kind, name) => {
                                let title = if kind == "publisher" { t!("Published by {name}", name = name) } else { t!("Developed by {name}", name = name) };
                                self.search_results(ui, &t, ctx, &title)
                            }
                            View::Browse => self.browse_page(ui, &t, ctx),
                            View::Wishlist => self.wishlist_page(ui, &t, ctx),
                            View::Cart => self.cart_page(ui, &t, ctx),
                            View::Discover => self.discover_page(ui, &t, ctx),
                            View::Points => self.points_page(ui, &t, ctx),
                            View::Offer(kind, id) => self.offer_page(ui, &t, ctx, kind, id),
                            View::Curators => self.curators_page(ui, &t, ctx),
                            View::Curator(id) => self.curator_page(ui, &t, ctx, id),
                            View::App(_) | View::Discovery(_) => {}
                        }
                        ui.add_space(t.dense(80.0));
                    });
                }
            }
        });
        self.flush_ratings();
        self.micro_end_frame();
        self.gift_dialog(ui, &t, ctx);
        self.cart_popup_ui(ui, &t, ctx);
        self.points_dialog(ui, &t, ctx);
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        self.store_bar(ui, ctx)
    }

    fn settings(&mut self, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        let t = ctx.theme.clone();
        ui.label(w::text_semibold(t!("Mature content"), 15.0, t.text));
        ui.label(w::text(t!("Choose which kinds of content the store shows. Hidden games don't appear in lists, and opening one asks first."), 12.5, t.text_dim));
        ui.add_space(8.0);
        for (id, label) in DESCRIPTORS {
            let mut show = !self.hidden.contains(id);
            if w::check(ui, &t, &mut show, w::text(i18n::tr_dyn(label), 14.0, t.text)).changed() {
                if show {
                    self.hidden.remove(id);
                } else {
                    self.hidden.insert(*id);
                }
            }
        }
        let mut ask = !self.never_ask;
        if w::check(ui, &t, &mut ask, w::text(t!("Warn me before opening a hidden game's page"), 14.0, t.text)).changed() {
            self.never_ask = !ask;
        }
        ui.add_space(14.0);
        ui.label(w::text_semibold(t!("Price history"), 15.0, t.text));
        ui.add(egui::Label::new(w::text(t!("Paste a free IsThereAnyDeal API key to see each game's price history and all-time low on its store page. Get one at isthereanydeal.com/apps/my."), 12.5, t.text_dim)).wrap());
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.itad_key).password(true).hint_text(t!("API key")).desired_width(320.0).margin(vec2(10.0, 8.0))).with_edit_menu();
            if r.changed() {
                self.history.clear();
            }
            if ui.link(w::text(t!("Get a key"), 12.5, t.accent)).clicked() {
                crate::links::open_http("https://isthereanydeal.com/apps/my/");
            }
        });
        true
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&Saved { hidden: self.hidden.iter().copied().collect(), never_ask: self.never_ask, itad_key: self.itad_key.clone(), price_log: self.price_log.clone() }).ok()
    }

    fn load(&mut self, data: &str) {
        if let Ok(saved) = serde_json::from_str::<Saved>(data) {
            self.hidden = saved.hidden.into_iter().collect();
            self.never_ask = saved.never_ask;
            self.itad_key = saved.itad_key;
            self.price_log = saved.price_log;
        } else if let Ok(h) = serde_json::from_str(data) {
            // Older format: just the hidden descriptors.
            self.hidden = h;
        }
    }
}

fn view_key(v: &View) -> String {
    match v {
        View::Home => "home".into(),
        View::Section(s) => format!("s:{s}"),
        View::Search(q) => format!("q:{q}"),
        View::Creator(k, n) => format!("c:{k}:{n}"),
        View::App(id) => format!("a:{id}"),
        View::Browse => "browse".into(),
        View::Wishlist => "wishlist".into(),
        View::Cart => "cart".into(),
        View::Discovery(i) => format!("d:{i}"),
        View::Discover => "discover".into(),
        View::Points => "points".into(),
        View::Offer(store::offers::OfferKind::Bundle, id) => format!("bundle:{id}"),
        View::Offer(store::offers::OfferKind::Package, id) => format!("sub:{id}"),
        View::Curators => "curators".into(),
        View::Curator(id) => format!("curator:{id}"),
    }
}

/// Padding that clamps i8 margins for very wide windows.
fn pad_margin(ui: &Ui, max: f32) -> f32 {
    ((ui.available_width() - max) / 2.0).max(28.0)
}

impl Store {
    fn front_data(&self) -> Option<Result<(Vec<Item>, Vec<Section>), String>> {
        match &*self.front.as_ref()?.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(d) => Some(Ok(d.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        }
    }

    /// The store's own bar, like the redesign's: a few plain tabs (the
    /// lists move into "Browse"), and Wishlist and Cart on the right. One
    /// line at any width: nothing runs into anything else.
    fn subnav(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        let width = ui.available_width();
        let (bar, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
        ui.painter().rect_filled(bar, t.corner(), t.chrome);
        ui.painter().rect_stroke(bar, t.corner(), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(6.0, 5.0))).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut row;
        ui.spacing_mut().item_spacing.x = 2.0;
        let sections = match self.front_data() {
            Some(Ok((_, sections))) => sections,
            _ => Vec::new(),
        };
        if nav_tab(ui, t, &t!("Home"), self.view == View::Home).clicked() {
            self.open(View::Home);
        }
        // Browse: every game with filters, or one of the store's lists.
        let in_list = matches!(&self.view, View::Browse | View::Section(_));
        let r = nav_tab(ui, t, &format!("{}  {}", t!("Browse|store"), icons::CARET_DOWN), in_list);
        egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(220.0).show(|ui| {
            if w::menu_item(ui, t, icons::SLIDERS, t!("All games, with filters")).clicked() {
                self.open(View::Browse);
                ui.close();
            }
            for sec in &sections {
                let icon = match sec.id.as_str() {
                    "specials" => icons::TAG,
                    "top_sellers" => icons::FIRE,
                    "new_releases" => icons::STAR,
                    _ => icons::CALENDAR,
                };
                if w::menu_item(ui, t, icon, &i18n::tr_dyn(&sec.title)).clicked() {
                    self.open(View::Section(sec.id.clone()));
                    ui.close();
                }
            }
        });
        if ctx.state.steam_id != 0 && nav_tab(ui, t, t!("Discover"), matches!(self.view, View::Discover | View::Discovery(_))).clicked() {
            self.open(View::Discover);
        }
        if nav_tab(ui, t, t!("Curators"), matches!(self.view, View::Curators | View::Curator(_))).clicked() {
            self.open(View::Curators);
        }
        if nav_tab(ui, t, t!("Points Shop"), self.view == View::Points).clicked() {
            self.open(View::Points);
        }
        if let Some((q, _)) = &self.search {
            let q = q.clone();
            // The last search, or the developer/publisher page you came from.
            let (label, view) = match q.split_once(':') {
                Some((kind, name)) if kind == "developer" || kind == "publisher" => (format!("{}  {name}", icons::USERS), View::Creator(kind.to_string(), name.to_string())),
                _ => (format!("{}  \"{q}\"", icons::SEARCH), View::Search(q.clone())),
            };
            if ui.available_width() > 320.0 && nav_tab(ui, t, &label, self.view == view).clicked() {
                self.open(view);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let cart = ctx.state.cart.as_ref().map(|c| c.len()).unwrap_or(0);
            let label = if cart > 0 { format!("{}  {} ({cart})", icons::CART, t!("Cart")) } else { format!("{}  {}", icons::CART, t!("Cart")) };
            if nav_tab(ui, t, &label, self.view == View::Cart).clicked() {
                self.open(View::Cart);
            }
            let wish = ctx.state.wishlist.as_ref().map(|w| w.len()).unwrap_or(0);
            let label = if wish > 0 { format!("{}  {} ({wish})", icons::HEART, t!("Wishlist")) } else { format!("{}  {}", icons::HEART, t!("Wishlist")) };
            if nav_tab(ui, t, &label, self.view == View::Wishlist).clicked() {
                self.open(View::Wishlist);
            }
        });
    }

    fn section(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: &str) {
        let Some(Ok((_, sections))) = self.front_data() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let Some(s) = sections.iter().find(|s| s.id == id) else { return };
        let items: Vec<Item> = s.items.iter().filter(|i| self.safe(i.app_id) == Some(true)).cloned().collect();
        w::page_title(ui, t, &i18n::tr_dyn(&s.title), Some(&t!("{n} games", n = items.len())));
        self.grid(ui, t, ctx, &items);
    }

    /// Store cards in rows, as many across as fit.
    fn grid(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[Item]) {
        let gap = t.dense(21.0);
        let per = (((ui.available_width() + gap) / (t.dense(330.0) + gap)).floor() as usize).max(1);
        let cw = ((ui.available_width() - gap * (per as f32 - 1.0)) / per as f32).floor();
        let wishlist = ctx.state.wishlist.clone().unwrap_or_default();
        for row in items.chunks(per) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for item in row {
                    let wished = wishlist.iter().any(|(a, _)| *a == item.app_id);
                    let out = home::store_card(ui, t, item, cw, self.owned.contains(&item.app_id), wished, ctx.state.steam_id != 0);
                    self.micro(ui, item.app_id, out.img, t.corner_small(), out.hovered);
                    if out.wish {
                        ctx.send(if wished { client::Command::WishlistRemove { app_id: item.app_id } } else { client::Command::WishlistAdd { app_id: item.app_id } });
                    } else if out.open {
                        self.open(View::App(item.app_id));
                    }
                }
            });
            ui.add_space(gap);
        }
    }

    fn search_results(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, title: &str) {
        w::page_title(ui, t, title, None);
        let Some((_, slot)) = &self.search else { return };
        let items = match &*slot.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(v) => Some(Ok(v.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        };
        match items {
            None => w::loading(ui, t, t!("Searching…")),
            Some(Err(e)) => w::empty(ui, t, icons::WARNING, t!("Search failed"), &e),
            Some(Ok(v)) if v.is_empty() => w::empty(ui, t, icons::SEARCH, t!("No results"), t!("Try a different name.")),
            Some(Ok(v)) => {
                let v: Vec<Item> = v.into_iter().filter(|i| self.safe(i.app_id) == Some(true)).collect();
                if v.is_empty() && !self.ratings_pending.is_empty() {
                    w::loading(ui, t, t!("Searching…"));
                }
                // Two columns of rows, like the home page's top sellers.
                let gap = t.dense(21.0);
                let per = if ui.available_width() > t.dense(900.0) { 2 } else { 1 };
                let cw = ((ui.available_width() - gap * (per as f32 - 1.0)) / per as f32).floor();
                let wishlist = ctx.state.wishlist.clone().unwrap_or_default();
                for row in v.chunks(per) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for item in row {
                            let wished = wishlist.iter().any(|(a, _)| *a == item.app_id);
                            let out = home::row_card(ui, t, item, cw, self.owned.contains(&item.app_id), wished, ctx.state.steam_id != 0);
                            if out.wish {
                                ctx.send(if wished { client::Command::WishlistRemove { app_id: item.app_id } } else { client::Command::WishlistAdd { app_id: item.app_id } });
                            } else if out.open {
                                self.open(View::App(item.app_id));
                            }
                        }
                    });
                    ui.add_space(t.dense(10.0));
                }
            }
        }
    }
}

/// Text that underlines on hover, like Steam's store links.
pub(super) fn link(ui: &mut Ui, t: &Theme, text: &str, size: f32, color: Color32) -> egui::Response {
    let r = ui.add(egui::Label::new(w::text(text, size, color)).sense(Sense::click()).selectable(false));
    if r.hovered() {
        ui.painter().hline(r.rect.x_range(), r.rect.bottom() - 1.0, Stroke::new(1.0, t.text));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "In library" ribbon on a store picture's top-left corner, like Steam's.
pub(super) fn in_library_badge(ui: &Ui, t: &Theme, img: Rect) {
    let text = format!("{}  {}", icons::CHECK, t!("In library"));
    let fg = t.text_on(t.play);
    let galley = ui.painter().layout_no_wrap(text, t.typography.caption.font(), fg);
    let r = Rect::from_min_size(img.min + vec2(t.spacing.sm, t.spacing.sm), galley.size() + vec2(14.0, 8.0));
    ui.painter().rect_filled(r, t.corner_small(), t.play);
    ui.painter().galley(r.min + vec2(7.0, 4.0), galley, fg);
}

/// Lay out store HTML / Steam BBCode blocks.
pub fn blocks(ui: &mut Ui, t: &Theme, blocks: &[Block], width: f32) {
    let job = |runs: &[(String, bool)], color: Color32| {
        let mut job = LayoutJob::default();
        for (i, (text, bold)) in runs.iter().enumerate() {
            let sep = if i > 0 { " " } else { "" };
            let font = if *bold { w::font_semibold(14.0) } else { w::font(14.0) };
            job.append(&format!("{sep}{text}"), 0.0, TextFormat { font_id: font, color, line_height: Some(21.0), ..Default::default() });
        }
        job.wrap.max_width = width;
        job
    };
    for b in blocks {
        match b {
            Block::Heading(s) => {
                ui.add_space(12.0);
                ui.label(w::text_bold(s.to_uppercase(), 13.5, t.text));
                ui.add_space(2.0);
            }
            Block::Paragraph(runs) => {
                ui.add(egui::Label::new(job(runs, t.text_dim)).wrap());
                ui.add_space(4.0);
            }
            Block::Bullet(runs) => {
                ui.horizontal_top(|ui| {
                    ui.label(w::text("•", 14.0, t.text_faint));
                    ui.add(egui::Label::new(job(runs, t.text_dim)).wrap());
                });
            }
            Block::Image(url) => {
                ui.add_space(6.0);
                let max_w = width.min(860.0);
                let sized = crate::sized_images::inline_url(url, max_w, 1200.0, ui.ctx().pixels_per_point());
                ui.add(egui::Image::new(sized).max_width(max_w).corner_radius(t.corner_small()));
                ui.add_space(6.0);
            }
            Block::Video(id) => {
                let Some(id) = crate::links::valid_youtube_id(id) else { continue };
                ui.add_space(6.0);
                let w_ = width.min(640.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w_, w_ * 0.5625), Sense::click());
                w::image_or_skeleton(ui, t, r, &format!("https://img.youtube.com/vi/{id}/hqdefault.jpg"), t.corner_small());
                ui.painter().rect_filled(r, t.corner_small(), Color32::from_black_alpha(60));
                ui.painter().circle_filled(r.center(), 30.0, Color32::from_rgb(0xE5, 0x2D, 0x27));
                w::icon_centered(ui.painter(), r.center(), icons::PLAY, 26.0, Color32::WHITE);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Watch on YouTube")).clicked() {
                    crate::links::open_youtube(id);
                }
                ui.add_space(6.0);
            }
        }
    }
}

const DESCRIPTORS: &[(u32, &str)] = &[
    (1, "Some nudity or sexual content"),
    (2, "Frequent violence or gore"),
    (3, "Adult-only sexual content"),
    (4, "Frequent nudity or sexual content"),
    (5, "General mature content"),
];

impl Store {
    /// True if the app's page should be gated by the user's content settings.
    fn details_mature(&mut self, id: AppId) -> bool {
        if self.never_ask {
            return false;
        }
        match self.safe(id) {
            Some(ok) => !ok,
            // Until we know, use the details call if it has already answered.
            None => self.details(id).is_some_and(|d| d.content_descriptors.iter().any(|x| self.hidden.contains(x))),
        }
    }

    /// Steam-style mature content notice instead of the page.
    fn mature_gate(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        let name = self.details(id).map(|d| d.name).unwrap_or_default();
        let notes = self.details(id).and_then(|d| d.content_notes);
        ui.add_space(80.0);
        w::centered(ui, 560.0, 32.0, |ui| {
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(w::text(icons::WARNING, 30.0, t.away));
                ui.label(w::text_bold(&name, 22.0, t.text));
                ui.add_space(4.0);
                ui.add(egui::Label::new(w::text(t!("This game contains content you chose to hide in your store preferences."), 14.0, t.text_dim)).wrap());
                if let Some(n) = notes {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(w::text(t!("Developer's description: {notes}", notes = n), 12.5, t.text_faint)).wrap());
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to the store"), w::Kind::Primary, w::Size::Md).clicked() {
                        self.open(View::Home);
                    }
                    if w::button(ui, t, None, t!("View page"), w::Kind::Secondary, w::Size::Md).clicked() {
                        self.allowed.insert(id);
                    }
                    if w::button(ui, t, None, t!("Don't ask again"), w::Kind::Ghost, w::Size::Md)
                        .on_hover_text(t!("Open these pages without this warning. You can turn it back on in the store's settings."))
                        .clicked()
                    {
                        self.never_ask = true;
                    }
                });
            });
        });
    }
}

/// The store link for a view, as `on_navigate` reads it.
fn view_arg(view: &View) -> Option<String> {
    match view {
        View::Home => None,
        View::Section(id) => Some(format!("section:{id}")),
        View::Search(q) => Some(format!("search:{q}")),
        View::Creator(kind, name) => Some(format!("{kind}:{name}")),
        View::App(id) => Some(format!("app:{id}")),
        View::Browse => Some("browse".into()),
        View::Wishlist => Some("wishlist".into()),
        View::Cart => Some("cart".into()),
        View::Discovery(i) => Some(format!("discovery:{i}")),
        View::Discover => Some("discover".into()),
        View::Points => Some("points".into()),
        View::Offer(store::offers::OfferKind::Bundle, id) => Some(format!("bundle:{id}")),
        View::Offer(store::offers::OfferKind::Package, id) => Some(format!("sub:{id}")),
        View::Curators => Some("curators".into()),
        View::Curator(id) => Some(format!("curator:{id}")),
    }
}

/// A tab on the store's bar: plain text, filled when it's the page you're on.
fn nav_tab(ui: &mut Ui, t: &Theme, label: &str, on: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), w::font_medium(13.5), t.text);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, 32.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let fill = if on { t.accent.gamma_multiply(0.22) } else { t.surface_hover.gamma_multiply(hover) };
    ui.painter().rect_filled(r, t.corner_small(), fill);
    let color = if on { t.text } else { w::mix(t.text_dim, t.text, hover) };
    ui.painter().galley(pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// For Big Picture: trailers, review summaries and a trailer player.
impl Store {
    pub(crate) fn trailers_for(&mut self, id: AppId) -> Vec<store::browse::Trailer> {
        self.browse_info(id).map(|b| b.trailers).unwrap_or_default()
    }

    /// (past month: up, down), (all time: percent positive, count).
    pub(crate) fn ratings_for(&mut self, id: AppId) -> (Option<(u64, u64)>, Option<(u32, u64)>) {
        let all = self.browse_info(id).and_then(|b| b.reviews).map(|r| (r.percent_positive, r.count as u64));
        (self.recent_votes(id).filter(|(u, d)| u + d > 0), all)
    }

    /// Play trailer `i` of a game in `rect` (started if it isn't the one
    /// playing). True when it ended.
    pub(crate) fn play_trailer(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, id: AppId, i: usize) -> bool {
        self.drawn_frame = self.frames;
        let trailers = self.trailers_for(id);
        let Some(tr) = trailers.get(i) else { return false };
        let Some(src) = tr.hls.clone().map(video::Source::Hls).or_else(|| tr.micro_mp4.clone().map(video::Source::Mp4)) else {
            w::image_or_skeleton(ui, t, rect, &tr.thumbnail, t.corner());
            return false;
        };
        if self.trailer.as_ref().is_none_or(|(url, _)| url != src.url()) {
            let url = src.url().to_string();
            let mut p = video::Player::new(&self.rt, ui.ctx(), src, video::widget::options(ui.ctx()));
            p.play();
            self.trailer = Some((url, p));
        }
        let (_, p) = self.trailer.as_mut().expect("just set");
        p.fullscreen = false;
        video::widget::show(ui, t, rect, p, &tr.thumbnail, &tr.name).ended
    }

    /// Pause/play, or move by `seconds`, the trailer playing.
    pub(crate) fn trailer_control(&mut self, toggle: bool, seconds: f64) {
        if let Some((_, p)) = self.trailer.as_mut() {
            if toggle {
                p.toggle();
            }
            if seconds != 0.0 {
                let to = (p.position() + seconds).clamp(0.0, p.duration().max(0.0));
                p.seek(to);
            }
        }
    }

    pub(crate) fn stop_trailer(&mut self) {
        self.trailer = None;
    }
}

//! Store: rendered natively from Steam's store JSON (no web view).
//! Payment is the only step that leaves the app: Steam only accepts it on its website.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2,
};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::AppId;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use store::browse::BrowseInfo;
use store::{Details, Item, Section, StoreClient, html::Block};
use ui::{Theme, icons, widgets as w};

#[path = "store_more.rs"]
mod more;

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}
type Shared<T> = Arc<Mutex<Load<T>>>;

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
}

pub struct Store {
    rt: tokio::runtime::Handle,
    egui: egui::Context,
    api: StoreClient,
    front: Option<Shared<(Vec<Item>, Vec<Section>)>>,
    details: HashMap<AppId, Shared<Details>>,
    browse: HashMap<AppId, Shared<BrowseInfo>>,
    tags: Option<Shared<HashMap<u32, String>>>,
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
    filters: store::query::Filters,
    browse_page: u32,
    query: Option<(
        store::query::Filters,
        u32,
        Shared<(Vec<store::query::QueryItem>, u64)>,
    )>,
    hide_owned: bool,
    wish_items: Option<(Vec<AppId>, Shared<Vec<store::query::QueryItem>>)>,
    cart_names: Option<(Vec<u32>, Shared<Vec<(u32, String, String)>>)>,
    gift: Option<more::Gift>,
    history: HashMap<AppId, Shared<store::itad::PriceHistory>>,
    itad_key: String,
    price_log: std::collections::BTreeMap<AppId, Vec<(u64, u64)>>,
    price_log_dirty: bool,
    lists_requested: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct Saved {
    hidden: Vec<u32>,
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
            tags: None,
            locale: ("us".into(), "english".into()),
            ratings: Default::default(),
            ratings_pending: Vec::new(),
            ratings_busy: Default::default(),
            hidden: [3, 4].into_iter().collect(),
            allowed: Default::default(),
            filters: Default::default(),
            browse_page: 0,
            query: None,
            hide_owned: false,
            wish_items: None,
            cart_names: None,
            gift: None,
            history: HashMap::new(),
            itad_key: String::new(),
            price_log: Default::default(),
            price_log_dirty: false,
            lists_requested: false,
            search: None,
            view: View::Home,
            hero: 0,
            hero_changed: 0.0,
            shelf_page: HashMap::new(),
            shot: 0,
            shot_centered: None,
            owned: Default::default(),
            trailer: None,
        }
    }

    fn fetch<T: Send + 'static>(
        &self,
        fut: impl Future<Output = store::Result<T>> + Send + 'static,
    ) -> Shared<T> {
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
            let slot = self.fetch(async move {
                api.browse(&[id])
                    .await
                    .map(|mut v| v.pop().unwrap_or_default())
            });
            self.browse.insert(id, slot);
        }
        match &*self.browse[&id].lock().unwrap() {
            Load::Ready(b) => Some(b.clone()),
            _ => None,
        }
    }

    fn tag_name(&mut self, id: u32) -> Option<String> {
        if self.tags.is_none() {
            let api = self.api.clone();
            self.tags = Some(self.fetch(async move { api.tag_names().await }));
        }
        match &*self.tags.as_ref()?.lock().unwrap() {
            Load::Ready(m) => m.get(&id).cloned(),
            _ => None,
        }
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
            self.tags = None;
            self.ratings.lock().unwrap().clear();
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
        let (api, out, busy, ctx) = (
            self.api.clone(),
            self.ratings.clone(),
            self.ratings_busy.clone(),
            self.egui.clone(),
        );
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
        if let View::App(_) = view {
            self.shot = 0;
            self.shot_centered = None;
        }
        self.view = view;
    }

    /// A developer's or publisher's games, like their store page.
    fn open_creator(&mut self, kind: &str, name: &str) {
        let (api, k, n) = (self.api.clone(), kind.to_string(), name.to_string());
        let slot = self.fetch(async move { api.creator(&k, &n).await });
        self.search = Some((format!("{kind}:{name}"), slot));
        self.open(View::Creator(kind.to_string(), name.to_string()));
    }

    /// Browse everything with one tag (clicked on a game page).
    fn open_tag(&mut self, tag: u32) {
        self.filters = store::query::Filters {
            tags: vec![tag],
            ..Default::default()
        };
        self.browse_page = 0;
        self.open(View::Browse);
    }

    /// A tag id from its (localized) name, e.g. a genre on a game page.
    fn tag_by_name(&mut self, name: &str) -> Option<u32> {
        self.tag_name(0);
        match &*self.tags.as_ref()?.lock().unwrap() {
            Load::Ready(m) => m
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(name))
                .map(|(id, _)| *id),
            _ => None,
        }
    }
}

impl Extension for Store {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "store",
            name: "Store",
            author: "SteamInRust",
            description: "The Steam store, rendered natively",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![
            PageSpec::new("store", "Store", icons::STORE)
                .main()
                .order(10),
        ]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        match arg {
            Some(a) if a.starts_with("search:") => {
                let q = a.trim_start_matches("search:").to_string();
                let api = self.api.clone();
                let qq = q.clone();
                let slot = self.fetch(async move { api.search(&qq).await });
                self.search = Some((q.clone(), slot));
                self.open(View::Search(q));
            }
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
            Some(a) => {
                if let Ok(id) = a.trim_start_matches("app:").parse() {
                    self.open(View::App(id));
                }
            }
            None => self.open(View::Home),
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        // Refresh the owned set only when the library changes size.
        let count = ctx.state.apps.as_ref().map(|a| a.len()).unwrap_or(0) + ctx.local.games.len();
        if count != self.owned.len() {
            self.owned = ctx
                .state
                .apps
                .as_ref()
                .map(|a| a.keys().copied().collect())
                .unwrap_or_default();
            self.owned.extend(ctx.local.games.iter().map(|g| g.app_id));
        }
        if !self.sync_locale(ctx) {
            // Don't load prices in the wrong currency while sign-in finishes.
            let (r, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            w::skeleton(ui, &t, r.shrink(28.0), t.corner());
            return;
        }
        if !self.lists_requested && ctx.state.steam_id != 0 {
            self.lists_requested = true;
            ctx.send(client::Command::LoadWishlist);
            ctx.send(client::Command::LoadCart);
        }
        if self.front.is_none() {
            let api = self.api.clone();
            self.front = Some(self.fetch(async move { api.front().await }));
        }
        egui::ScrollArea::vertical()
            .id_salt(("store", view_key(&self.view)))
            .auto_shrink([false, false])
            .show(ui, |ui| match self.view.clone() {
                View::App(id) if !self.allowed.contains(&id) && self.details_mature(id) => {
                    self.mature_gate(ui, &t, id)
                }
                View::App(id) => self.app_page(ui, &t, ctx, id),
                view => {
                    ui.add_space(20.0);
                    w::centered(ui, 1200.0, 28.0, |ui| {
                        self.subnav(ui, &t, ctx);
                        ui.add_space(18.0);
                        match view {
                            View::Home => self.home(ui, &t),
                            View::Section(id) => self.section(ui, &t, &id),
                            View::Search(q) => {
                                self.search_results(ui, &t, &t!("Results for “{q}”", q = q))
                            }
                            View::Creator(kind, name) => {
                                let title = if kind == "publisher" {
                                    t!("Published by {name}", name = name)
                                } else {
                                    t!("Developed by {name}", name = name)
                                };
                                self.search_results(ui, &t, &title)
                            }
                            View::Browse => self.browse_page(ui, &t, ctx),
                            View::Wishlist => self.wishlist_page(ui, &t, ctx),
                            View::Cart => self.cart_page(ui, &t, ctx),
                            View::App(_) => {}
                        }
                        ui.add_space(40.0);
                    });
                }
            });
        self.flush_ratings();
        self.gift_dialog(ui, &t, ctx);
    }

    fn settings(&mut self, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        let t = ctx.theme.clone();
        ui.label(w::text_semibold(t!("Mature content"), 15.0, t.text));
        ui.label(w::text(t!("Choose which kinds of content the store shows. Hidden games don't appear in lists, and opening one asks first."), 12.5, t.text_dim));
        ui.add_space(8.0);
        for (id, label) in DESCRIPTORS {
            let mut show = !self.hidden.contains(id);
            if w::check(
                ui,
                &t,
                &mut show,
                w::text(i18n::tr_dyn(label), 14.0, t.text),
            )
            .changed()
            {
                if show {
                    self.hidden.remove(id);
                } else {
                    self.hidden.insert(*id);
                }
            }
        }
        ui.add_space(14.0);
        ui.label(w::text_semibold(t!("Price history"), 15.0, t.text));
        ui.add(egui::Label::new(w::text(t!("Paste a free IsThereAnyDeal API key to see each game's price history and all-time low on its store page. Get one at isthereanydeal.com/apps/my."), 12.5, t.text_dim)).wrap());
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.itad_key)
                    .password(true)
                    .hint_text(t!("API key"))
                    .desired_width(320.0)
                    .margin(vec2(10.0, 8.0)),
            );
            if r.changed() {
                self.history.clear();
            }
            if ui.link(w::text(t!("Get a key"), 12.5, t.accent)).clicked() {
                let _ = open::that_detached("https://isthereanydeal.com/apps/my/");
            }
        });
        true
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&Saved {
            hidden: self.hidden.iter().copied().collect(),
            itad_key: self.itad_key.clone(),
            price_log: self.price_log.clone(),
        })
        .ok()
    }

    fn load(&mut self, data: &str) {
        if let Ok(saved) = serde_json::from_str::<Saved>(data) {
            self.hidden = saved.hidden.into_iter().collect();
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

    fn subnav(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if w::chip(
                ui,
                t,
                &format!("{}  {}", icons::SPARKLE, t!("Featured")),
                self.view == View::Home,
            )
            .clicked()
            {
                self.open(View::Home);
            }
            if w::chip(
                ui,
                t,
                &format!("{}  {}", icons::SLIDERS, t!("Browse")),
                self.view == View::Browse,
            )
            .clicked()
            {
                self.open(View::Browse);
            }
            if let Some(Ok((_, sections))) = self.front_data() {
                for s in &sections {
                    let icon = match s.id.as_str() {
                        "specials" => icons::TAG,
                        "top_sellers" => icons::FIRE,
                        "new_releases" => icons::STAR,
                        _ => icons::CALENDAR,
                    };
                    if w::chip(
                        ui,
                        t,
                        &format!("{icon}  {}", i18n::tr_dyn(&s.title)),
                        self.view == View::Section(s.id.clone()),
                    )
                    .clicked()
                    {
                        self.open(View::Section(s.id.clone()));
                    }
                }
            }
            if let Some((q, _)) = &self.search {
                let q = q.clone();
                // The last search, or the developer/publisher page you came from.
                let (label, view) = match q.split_once(':') {
                    Some((kind, name)) if kind == "developer" || kind == "publisher" => (
                        format!("{}  {name}", icons::USERS),
                        View::Creator(kind.to_string(), name.to_string()),
                    ),
                    _ => (
                        format!("{}  \"{q}\"", icons::SEARCH),
                        View::Search(q.clone()),
                    ),
                };
                if w::chip(ui, t, &label, self.view == view).clicked() {
                    self.open(view);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let cart = ctx.state.cart.as_ref().map(|c| c.len()).unwrap_or(0);
                if w::chip(
                    ui,
                    t,
                    &format!("{}  {} ({cart})", icons::CART, t!("Cart")),
                    self.view == View::Cart,
                )
                .clicked()
                {
                    self.open(View::Cart);
                }
                let wish = ctx.state.wishlist.as_ref().map(|w| w.len()).unwrap_or(0);
                if w::chip(
                    ui,
                    t,
                    &format!("{}  {} ({wish})", icons::HEART, t!("Wishlist")),
                    self.view == View::Wishlist,
                )
                .clicked()
                {
                    self.open(View::Wishlist);
                }
            });
        });
    }

    fn home(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(data) = self.front_data() else {
            let w_ = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w_, 380.0), Sense::hover());
            w::skeleton(ui, t, r, t.corner());
            return;
        };
        let (spotlight, sections) = match data {
            Ok(d) => d,
            Err(e) => {
                w::empty(ui, t, icons::WARNING, t!("The store didn't load"), &e);
                if w::button(
                    ui,
                    t,
                    Some(icons::REFRESH),
                    t!("Try again"),
                    w::Kind::Secondary,
                    w::Size::Md,
                )
                .clicked()
                {
                    self.front = None;
                }
                return;
            }
        };
        // Hide items the user filtered out; wait for ratings before showing art.
        let spotlight: Vec<Item> = spotlight
            .into_iter()
            .filter(|i| self.safe(i.app_id) == Some(true))
            .collect();
        let sections: Vec<Section> = sections
            .into_iter()
            .map(|mut s| {
                s.items.retain(|i| self.safe(i.app_id) != Some(false));
                s
            })
            .collect();
        if spotlight.is_empty() && !self.ratings_pending.is_empty() {
            let w_ = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w_, 400.0), Sense::hover());
            w::skeleton(ui, t, r, t.corner());
            ui.add_space(32.0);
        }
        if !spotlight.is_empty() {
            self.carousel(ui, t, &spotlight);
            ui.add_space(32.0);
        }
        for s in &sections {
            self.shelf(ui, t, s);
            ui.add_space(28.0);
        }
    }

    fn carousel(&mut self, ui: &mut Ui, t: &Theme, items: &[Item]) {
        let now = ui.input(|i| i.time);
        let n = items.len();
        self.hero %= n;
        let item = items[self.hero].clone();
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 400.0), Sense::click());
        if !resp.hovered() && now - self.hero_changed > 8.0 {
            self.hero = (self.hero + 1) % n;
            self.hero_changed = now;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));

        let p = ui.painter();
        p.rect_filled(rect, t.corner(), t.surface);
        let img = Rect::from_min_size(rect.min, vec2(rect.width() * 0.64, rect.height()));
        let r_img = CornerRadius {
            nw: t.radius,
            sw: t.radius,
            ne: 0,
            se: 0,
        };
        w::image_or_skeleton(
            ui,
            t,
            img,
            item.large.as_deref().unwrap_or(&item.header),
            r_img,
        );
        let owned = self.owned.contains(&item.app_id);
        if owned {
            in_library_badge(ui, t, img);
        }

        // Right panel: title, screenshots, price.
        let side =
            Rect::from_min_max(pos2(img.right(), rect.top()), rect.max).shrink2(vec2(22.0, 22.0));
        // Everything above the price row, so nothing can run into it.
        let bottom = Rect::from_min_max(pos2(side.left(), side.bottom() - 30.0), side.max);
        let upper = Rect::from_min_max(side.min, pos2(side.right(), bottom.top() - 12.0));
        let mut c = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(upper)
                .layout(Layout::top_down(Align::Min)),
        );
        c.set_clip_rect(upper.expand(1.0));
        c.add(egui::Label::new(w::text_bold(&item.name, 22.0, t.text)).truncate());
        c.add_space(8.0);
        if let Some(d) = self.details(item.app_id) {
            // Screenshots: two rows if they fit above the tags, else one.
            let tags_h = 30.0;
            let room = (upper.bottom() - c.cursor().top() - tags_h - 10.0).max(0.0);
            let mut tw = (side.width() - 8.0) / 2.0;
            let rows = if 2.0 * tw * 0.5625 + 8.0 <= room {
                2
            } else {
                1
            };
            if rows == 1 && tw * 0.5625 > room {
                tw = room / 0.5625;
            }
            for row in d.screenshots.chunks(2).take(rows) {
                c.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for s in row {
                        let (r, _) = ui.allocate_exact_size(vec2(tw, tw * 0.5625), Sense::hover());
                        w::image_or_skeleton(ui, t, r, &s.thumb, t.corner_small());
                    }
                });
            }
            c.add_space(10.0);
            // One row of genre pills: stop at the first one that doesn't fit.
            c.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for g in d.genres.iter().take(4) {
                    let need = ui
                        .painter()
                        .layout_no_wrap(g.clone(), w::font(12.0), t.text_dim)
                        .size()
                        .x
                        + 22.0;
                    if need > ui.available_width() {
                        break;
                    }
                    w::pill(ui, g, t.text_dim, t.raised);
                }
            });
        } else {
            let tw = side.width();
            let (r, _) = c.allocate_exact_size(
                vec2(tw, (tw * 0.56).min(upper.height() - 40.0).max(0.0)),
                Sense::hover(),
            );
            w::skeleton(&c, t, r, t.corner_small());
        }
        // Price on the right first; the label gets what's left and shortens itself.
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bottom)
                .layout(Layout::right_to_left(Align::Center)),
        );
        if owned {
            b.label(w::text_semibold(t!("In library"), 14.0, t.play));
        } else {
            w::price_tag(
                &mut b,
                t,
                &item.price.label(),
                item.price.original_label().as_deref(),
                item.price.discount,
            );
        }
        b.add_space(10.0);
        b.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let label = if item.price.discount > 0 {
                t!("Special offer")
            } else {
                t!("Now available")
            };
            ui.add(egui::Label::new(w::text_medium(label, 13.0, t.text_dim)).truncate());
        });

        // Arrows and dots.
        for (dir, x) in [(-1i32, rect.left() - 18.0), (1, rect.right() + 18.0)] {
            let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(32.0, 60.0));
            let a = ui.interact(r, egui::Id::unique(("hero-arrow", dir)), Sense::click());
            let col = if a.hovered() { t.text } else { t.text_faint };
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                if dir < 0 {
                    icons::BACK
                } else {
                    icons::CARET_RIGHT
                },
                w::font(26.0),
                col,
            );
            if a.clicked() {
                self.hero = (self.hero as i32 + dir).rem_euclid(n as i32) as usize;
                self.hero_changed = now;
            }
        }
        let dots_w = n as f32 * 18.0;
        for i in 0..n {
            let c = pos2(
                rect.center().x - dots_w / 2.0 + i as f32 * 18.0 + 9.0,
                rect.bottom() + 16.0,
            );
            let r = Rect::from_center_size(c, vec2(14.0, 14.0));
            let d = ui.interact(r, egui::Id::unique(("hero-dot", i)), Sense::click());
            let on = i == self.hero;
            ui.painter().rect_filled(
                Rect::from_center_size(c, vec2(if on { 18.0 } else { 8.0 }, 4.0)),
                CornerRadius::same(2),
                if on { t.accent } else { t.border },
            );
            if d.clicked() {
                self.hero = i;
                self.hero_changed = now;
            }
        }
        ui.add_space(14.0);
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.open(View::App(item.app_id));
        }
    }

    fn shelf(&mut self, ui: &mut Ui, t: &Theme, s: &Section) {
        let card_w = 272.0;
        let gap = 16.0;
        let per = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
        let pages = s.items.len().div_ceil(per);
        let page = self
            .shelf_page
            .get(&s.id)
            .copied()
            .unwrap_or(0)
            .min(pages.saturating_sub(1));
        let mut see_all = false;
        ui.horizontal(|ui| {
            ui.label(w::text_bold(&s.title, 20.0, t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(page + 1 < pages, |ui| {
                    if w::icon_button(ui, t, icons::CARET_RIGHT, t!("Next"), false, 0).clicked() {
                        self.shelf_page.insert(s.id.clone(), page + 1);
                    }
                });
                ui.add_enabled_ui(page > 0, |ui| {
                    if w::icon_button(ui, t, icons::BACK, t!("Previous"), false, 0).clicked() {
                        self.shelf_page.insert(s.id.clone(), page - 1);
                    }
                });
                ui.add_space(8.0);
                see_all =
                    w::button(ui, t, None, t!("See all"), w::Kind::Ghost, w::Size::Sm).clicked();
            });
        });
        ui.add_space(8.0);
        let actual_w = (ui.available_width() - gap * (per as f32 - 1.0)) / per as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for item in s.items.iter().skip(page * per).take(per) {
                if self.safe(item.app_id).is_none() {
                    let h = actual_w * 215.0 / 460.0 + 64.0;
                    let (r, _) = ui.allocate_exact_size(vec2(actual_w, h), Sense::hover());
                    w::skeleton(ui, t, r, t.corner());
                    continue;
                }
                if card(ui, t, item, actual_w, self.owned.contains(&item.app_id)).clicked() {
                    self.open(View::App(item.app_id));
                }
            }
        });
        if see_all {
            self.open(View::Section(s.id.clone()));
        }
    }

    fn section(&mut self, ui: &mut Ui, t: &Theme, id: &str) {
        let Some(Ok((_, sections))) = self.front_data() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let Some(s) = sections.iter().find(|s| s.id == id) else {
            return;
        };
        let items: Vec<Item> = s
            .items
            .iter()
            .filter(|i| self.safe(i.app_id) == Some(true))
            .cloned()
            .collect();
        w::page_title(
            ui,
            t,
            &i18n::tr_dyn(&s.title),
            Some(&t!("{n} games", n = items.len())),
        );
        self.grid(ui, t, &items);
    }

    fn grid(&mut self, ui: &mut Ui, t: &Theme, items: &[Item]) {
        let gap = 16.0;
        let per = (((ui.available_width() + gap) / (272.0 + gap)).floor() as usize).max(1);
        let cw = (ui.available_width() - gap * (per as f32 - 1.0)) / per as f32;
        for row in items.chunks(per) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for item in row {
                    if card(ui, t, item, cw, self.owned.contains(&item.app_id)).clicked() {
                        self.open(View::App(item.app_id));
                    }
                }
            });
            ui.add_space(gap);
        }
    }

    fn search_results(&mut self, ui: &mut Ui, t: &Theme, title: &str) {
        w::page_title(ui, t, title, None);
        let Some((_, slot)) = &self.search else {
            return;
        };
        let items = match &*slot.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(v) => Some(Ok(v.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        };
        match items {
            None => w::loading(ui, t, t!("Searching…")),
            Some(Err(e)) => w::empty(ui, t, icons::WARNING, t!("Search failed"), &e),
            Some(Ok(v)) if v.is_empty() => w::empty(
                ui,
                t,
                icons::SEARCH,
                t!("No results"),
                t!("Try a different name."),
            ),
            Some(Ok(v)) => {
                let v: Vec<Item> = v
                    .into_iter()
                    .filter(|i| self.safe(i.app_id) == Some(true))
                    .collect();
                if v.is_empty() && !self.ratings_pending.is_empty() {
                    w::loading(ui, t, t!("Searching…"));
                }
                for item in &v {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::click());
                    let hover = ui
                        .ctx()
                        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    ui.painter().rect_filled(
                        rect,
                        t.corner(),
                        w::mix(t.surface, t.surface_hover, hover),
                    );
                    let img = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(137.0, 64.0));
                    w::image_or_skeleton(ui, t, img, &item.header, t.corner_small());
                    let mut c = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(
                                pos2(img.right() + 16.0, rect.top()),
                                rect.max - vec2(16.0, 0.0),
                            ))
                            .layout(Layout::left_to_right(Align::Center)),
                    );
                    c.vertical(|ui| {
                        ui.add_space(18.0);
                        ui.label(w::text_semibold(&item.name, 16.0, t.text));
                        if item.windows {
                            ui.label(w::text(icons::WINDOWS, 13.0, t.text_faint));
                        }
                    });
                    c.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.owned.contains(&item.app_id) {
                            w::pill(ui, t!("In library"), t.play, t.play.gamma_multiply(0.15));
                        } else {
                            w::price_tag(
                                ui,
                                t,
                                &item.price.label(),
                                item.price.original_label().as_deref(),
                                item.price.discount,
                            );
                        }
                    });
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.open(View::App(item.app_id));
                    }
                    ui.add_space(8.0);
                }
            }
        }
    }

    fn app_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        let max = 1200.0;
        let pad = pad_margin(ui, max);
        let state = self.details.get(&id).map(|s| match &*s.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(d) => Some(Ok(d.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        });
        let d = match state.flatten() {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                egui::Frame::new()
                    .inner_margin(Margin::same(32))
                    .show(ui, |ui| {
                        if w::button(
                            ui,
                            t,
                            Some(icons::ARROW_LEFT),
                            t!("Store"),
                            w::Kind::Ghost,
                            w::Size::Sm,
                        )
                        .clicked()
                        {
                            self.open(View::Home);
                        }
                        w::empty(ui, t, icons::WARNING, t!("This game isn't available"), &e);
                    });
                return;
            }
            None => {
                self.details(id);
                w::loading(ui, t, t!("Loading game…"));
                return;
            }
        };

        // Background art fading into the page.
        let full = ui.max_rect();
        let bg = Rect::from_min_size(full.min, vec2(full.width(), 560.0));
        if w::paint_cover(ui, bg, &d.background, CornerRadius::ZERO) {
            ui.painter()
                .rect_filled(bg, CornerRadius::ZERO, t.bg.gamma_multiply(0.55));
        }
        w::fade_down(ui, bg, t.bg, 0.0);

        ui.add_space(20.0);
        w::centered(ui, max, pad.min(28.0), |ui| {
            ui.horizontal(|ui| {
                if w::button(
                    ui,
                    t,
                    Some(icons::ARROW_LEFT),
                    t!("Store"),
                    w::Kind::Ghost,
                    w::Size::Sm,
                )
                .clicked()
                {
                    self.open(View::Home);
                }
                if let Some(g) = d.genres.first() {
                    ui.label(w::text(icons::CARET_RIGHT, 13.0, t.text_dim));
                    if link(ui, t, g, 13.0, t.text_dim).clicked() {
                        if let Some(tag) = self.tag_by_name(g) {
                            self.open_tag(tag);
                        }
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(w::text_bold(&d.name, 34.0, Color32::WHITE));
            ui.add_space(14.0);

            let total = ui.available_width();
            let right_w = (total * 0.34).clamp(300.0, 420.0);
            let left_w = total - right_w - 24.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                ui.vertical(|ui| {
                    ui.set_width(left_w);
                    let trailers = self.browse_info(id).map(|b| b.trailers).unwrap_or_default();
                    self.media(ui, t, &d, &trailers, left_w);
                });
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    let (r, _) = ui.allocate_exact_size(
                        vec2(right_w, right_w * 215.0 / 460.0),
                        Sense::hover(),
                    );
                    w::image_or_skeleton(ui, t, r, &d.header, t.corner_small());
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(w::text(&d.short_description, 14.0, t.text_dim)).wrap(),
                    );
                    ui.add_space(10.0);
                    let info = self.browse_info(id);
                    egui::Grid::new("app-facts")
                        .num_columns(2)
                        .spacing(vec2(14.0, 6.0))
                        .show(ui, |ui| {
                            if let Some(r) = info.as_ref().and_then(|i| i.reviews.clone()) {
                                ui.label(w::text_bold(t!("REVIEWS"), 10.5, t.text_faint));
                                ui.horizontal(|ui| {
                                    ui.label(w::text_semibold(
                                        i18n::tr_dyn(&r.label),
                                        13.5,
                                        review_color(t, r.score),
                                    ));
                                    ui.label(w::text(
                                        t!(
                                            "({percent}% of {count})",
                                            percent = r.percent_positive,
                                            count = thousands(r.count)
                                        ),
                                        12.5,
                                        t.text_faint,
                                    ));
                                });
                                ui.end_row();
                            } else if let Some(r) = d.recommendations {
                                fact(
                                    ui,
                                    t,
                                    t!("REVIEWS"),
                                    &format!("{} {}", icons::THUMBS_UP, thousands(r)),
                                );
                            }
                            if let Some(m) = d.metacritic {
                                ui.label(w::text_bold(t!("METACRITIC"), 10.5, t.text_faint));
                                let col = if m >= 75 {
                                    Color32::from_rgb(0x66, 0xCC, 0x33)
                                } else if m >= 50 {
                                    t.away
                                } else {
                                    t.danger
                                };
                                w::pill(ui, &m.to_string(), Color32::BLACK, col);
                                ui.end_row();
                            }
                            fact(ui, t, t!("Release"), &d.release_date);
                            for (label, kind, names) in [
                                (t!("Developer"), "developer", &d.developers),
                                (t!("Publisher"), "publisher", &d.publishers),
                            ] {
                                if names.is_empty() {
                                    continue;
                                }
                                ui.label(w::text_bold(label.to_uppercase(), 10.5, t.text_faint));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for (i, name) in names.iter().enumerate() {
                                        if i > 0 {
                                            ui.label(w::text(", ", 13.0, t.text_dim));
                                        }
                                        if link(ui, t, name, 13.0, t.accent)
                                            .on_hover_text(t!("All games by {name}", name = name))
                                            .clicked()
                                        {
                                            self.open_creator(kind, name);
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        // Popular tags (or the genres before tags load); each opens Browse with it.
                        let tags: Vec<(Option<u32>, String)> = info
                            .as_ref()
                            .map(|i| {
                                i.tags
                                    .iter()
                                    .take(8)
                                    .filter_map(|id| Some((Some(*id), self.tag_name(*id)?)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let list: Vec<(Option<u32>, String)> = if tags.is_empty() {
                            d.genres.iter().map(|g| (None, g.clone())).collect()
                        } else {
                            tags
                        };
                        for (id, name) in &list {
                            if tag_chip(ui, t, name).clicked() {
                                if let Some(tag) = id.or_else(|| self.tag_by_name(name)) {
                                    self.open_tag(tag);
                                }
                            }
                        }
                    });
                });
            });
            ui.add_space(28.0);

            // Purchase box.
            let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&id))
                || ctx.local.games.iter().any(|g| g.app_id == id);
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let title = if d.is_free { t!("Play {name}", name = d.name) } else { t!("Buy {name}", name = d.name) };
                        ui.label(w::text_bold(title, 18.0, t.text));
                        let (win, mac, lin) = d.platforms;
                        let mut plats = String::new();
                        for (on, i) in [(win, icons::WINDOWS), (mac, icons::APPLE), (lin, icons::LINUX)] {
                            if on {
                                plats.push_str(i);
                                plats.push_str("  ");
                            }
                        }
                        ui.label(w::text(plats, 15.0, t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if owned {
                            if w::button(ui, t, Some(icons::BOOKS), t!("In your library"), w::Kind::Play, w::Size::Lg).clicked() {
                                ctx.navigate("library", Some(id.to_string()));
                            }
                            // Like Steam: you can still buy it, as a gift for a friend.
                            if let Some(pkg) = d.package_id.filter(|_| !d.is_free) {
                                ui.add_space(8.0);
                                if w::button(ui, t, Some(icons::USER), t!("Buy as a gift…"), w::Kind::Secondary, w::Size::Lg)
                                    .on_hover_text(t!("You already own this; buying it again makes a gift for a friend."))
                                    .clicked()
                                {
                                    self.gift = Some(more::Gift { app: id, package: pkg, name: d.name.clone(), to: None, search: String::new(), checks: Default::default() });
                                }
                            }
                        } else if d.is_free {
                            if let Some(pkg) = d.package_id {
                                if w::button(ui, t, Some(icons::PLUS), t!("Add to library"), w::Kind::Play, w::Size::Lg).on_hover_text(t!("Free to play: adds it to your account through the cart")).clicked() {
                                    ctx.send(client::Command::CartAdd { package_id: pkg, gift_to: None });
                                    let _ = open::that_detached("https://store.steampowered.com/cart");
                                }
                            }
                        } else if d.coming_soon {
                            w::pill(ui, t!("Coming soon"), t.text, t.raised);
                            ui.add_space(8.0);
                            self.purchase_actions(ui, t, ctx, id, None, &d.name);
                        } else {
                            self.purchase_actions(ui, t, ctx, id, d.package_id, &d.name);
                            ui.add_space(8.0);
                            if let Some(p) = &d.price {
                                if let Some(c) = p.final_ {
                                    self.log_price(id, c);
                                }
                            }
                            match &d.price {
                                Some(p) => w::price_tag(ui, t, &p.label(), p.original_label().as_deref(), p.discount),
                                None => {
                                    ui.label(w::text_semibold(d.price_text.clone().unwrap_or_default(), 15.0, t.text));
                                }
                            }
                        }
                    });
                });
            });
            ui.add_space(28.0);

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                ui.vertical(|ui| {
                    ui.set_width(left_w);
                    if !d.is_free {
                        self.price_history(ui, t, id);
                    }
                    w::section(ui, t, t!("About this game"), None);
                    blocks(ui, t, &d.about, left_w);
                    ui.add_space(24.0);
                    w::section(ui, t, t!("System requirements"), None);
                    ui.columns(2, |cols| {
                        cols[0].label(w::text_semibold(t!("Minimum"), 14.0, t.text_dim));
                        blocks(&mut cols[0], t, &d.requirements_min, left_w / 2.0);
                        cols[1].label(w::text_semibold(t!("Recommended"), 14.0, t.text_dim));
                        blocks(&mut cols[1], t, &d.requirements_rec, left_w / 2.0);
                    });
                });
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    self.relevance(ui, t, ctx, id, &d);
                    ui.add_space(12.0);
                    if !d.categories.is_empty() {
                        w::card(t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(w::text_semibold(t!("Features"), 14.0, t.text));
                            ui.add_space(4.0);
                            for c in &d.categories {
                                ui.horizontal(|ui| {
                                    ui.label(w::text(feature_icon(c), 15.0, t.accent));
                                    ui.label(w::text(c, 13.5, t.text_dim));
                                });
                            }
                        });
                        ui.add_space(12.0);
                    }
                    w::card(t).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(w::text_semibold(t!("Languages"), 14.0, t.text));
                        ui.add(
                            egui::Label::new(w::text(
                                d.languages
                                    .replace("*languages with full audio support", ""),
                                12.5,
                                t.text_dim,
                            ))
                            .wrap(),
                        );
                        if let Some(site) = &d.website {
                            ui.add_space(8.0);
                            if ui
                                .link(w::text(
                                    format!("{}  {}", icons::GLOBE, t!("Official website")),
                                    13.0,
                                    t.accent,
                                ))
                                .clicked()
                            {
                                let _ = open::that_detached(site);
                            }
                        }
                    });
                });
            });
        });
    }

    fn media(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        d: &Details,
        trailers: &[store::browse::Trailer],
        width: f32,
    ) {
        // Trailers come first in the strip, like on Steam. `self.shot` counts
        // trailers first, then screenshots.
        let n_tr = trailers.len();
        let total = n_tr + d.screenshots.len();
        if total == 0 {
            let (r, _) = ui.allocate_exact_size(vec2(width, width * 0.5625), Sense::hover());
            w::image_or_skeleton(ui, t, r, &d.header, t.corner());
            return;
        }
        self.shot %= total;
        let (main, resp) = ui.allocate_exact_size(vec2(width, width * 0.5625), Sense::click());
        if self.shot >= n_tr {
            // Leaving the trailers stops playback.
            self.trailer = None;
        }
        if self.shot < n_tr {
            let tr = &trailers[self.shot];
            let source = tr
                .hls
                .clone()
                .map(video::Source::Hls)
                .or_else(|| tr.micro_mp4.clone().map(video::Source::Mp4));
            if let Some(src) = source {
                if self
                    .trailer
                    .as_ref()
                    .is_none_or(|(url, _)| url != src.url())
                {
                    // A picked trailer plays right away (muted until the user
                    // turns sound on once), like Steam; full screen carries over.
                    let fullscreen = self.trailer.as_ref().is_some_and(|(_, p)| p.fullscreen);
                    let url = src.url().to_string();
                    let mut p = video::Player::new(
                        &self.rt,
                        ui.ctx(),
                        src,
                        video::widget::options(ui.ctx()),
                    );
                    p.fullscreen = fullscreen;
                    self.trailer = Some((url, p));
                }
                let (_, p) = self.trailer.as_mut().expect("just set");
                if video::widget::show(ui, t, main, p, &tr.thumbnail, &tr.name).ended {
                    // Like Steam: on to the next trailer or screenshot.
                    self.step_shot(1, total);
                }
            } else {
                w::image_or_skeleton(ui, t, main, &tr.thumbnail, t.corner());
                ui.painter().text(
                    main.left_bottom() + vec2(16.0, -16.0),
                    Align2::LEFT_BOTTOM,
                    &tr.name,
                    w::font_semibold(15.0),
                    Color32::WHITE,
                );
            }
        } else {
            let s = &d.screenshots[self.shot - n_tr];
            if !w::paint_cover(ui, main, &s.full, t.corner()) {
                w::image_or_skeleton(ui, t, main, &s.thumb, t.corner());
            }
        }
        // Previous / next on the sides while the pointer is over it, like Steam.
        if total > 1 && ui.rect_contains_pointer(main) {
            for (dir, icon, x) in [
                (-1i32, icons::CARET_LEFT, main.left() + 30.0),
                (1, icons::CARET_RIGHT, main.right() - 30.0),
            ] {
                let r = Rect::from_center_size(pos2(x, main.center().y), vec2(44.0, 64.0));
                let b = ui.interact(r, ui.scope_id().with(("media-arrow", dir)), Sense::click());
                ui.painter().rect_filled(
                    r,
                    CornerRadius::same(6),
                    Color32::from_black_alpha(if b.hovered() { 200 } else { 130 }),
                );
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    icon,
                    w::font(24.0),
                    Color32::WHITE,
                );
                if b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.step_shot(dir, total);
                }
            }
        } else if resp.clicked() && self.shot >= n_tr {
            self.step_shot(1, total);
        }
        ui.add_space(8.0);
        self.strip(ui, t, d, trailers, width);
    }

    /// Move the highlight by one, wrapping around.
    fn step_shot(&mut self, dir: i32, total: usize) {
        self.shot = (self.shot as i32 + dir).rem_euclid(total as i32) as usize;
    }

    /// The thumbnail strip: arrows at both ends, and the picked thumbnail
    /// scrolls to the middle, like Steam's.
    fn strip(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        d: &Details,
        trailers: &[store::browse::Trailer],
        width: f32,
    ) {
        let n_tr = trailers.len();
        let total = n_tr + d.screenshots.len();
        let arrow_w = 28.0;
        let center_now = self.shot_centered != Some(self.shot);
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let arrow = |ui: &mut Ui, icon: &str| -> bool {
                let (r, b) = ui.allocate_exact_size(vec2(arrow_w, 72.0), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(b.id, b.hovered(), 0.1);
                ui.painter().rect_filled(
                    r,
                    t.corner_small(),
                    w::mix(t.raised, t.surface_hover, hover),
                );
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    icon,
                    w::font(18.0),
                    w::mix(t.text_dim, t.text, hover),
                );
                b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            };
            if arrow(ui, icons::CARET_LEFT) && total > 0 {
                pick = Some((self.shot + total - 1) % total);
            }
            let inner = (width - 2.0 * (arrow_w + 6.0)).max(100.0);
            ui.allocate_ui(vec2(inner, 80.0), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("shots")
                    .max_width(inner)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            for i in 0..total {
                                let (r, resp) =
                                    ui.allocate_exact_size(vec2(128.0, 72.0), Sense::click());
                                if i < n_tr {
                                    let tr = &trailers[i];
                                    w::image_or_skeleton(ui, t, r, &tr.thumbnail, t.corner_small());
                                    ui.painter().circle_filled(
                                        r.center(),
                                        14.0,
                                        Color32::from_black_alpha(150),
                                    );
                                    ui.painter().text(
                                        r.center() + vec2(1.5, 0.0),
                                        Align2::CENTER_CENTER,
                                        icons::PLAY,
                                        w::font(14.0),
                                        Color32::WHITE,
                                    );
                                } else {
                                    w::image_or_skeleton(
                                        ui,
                                        t,
                                        r,
                                        &d.screenshots[i - n_tr].thumb,
                                        t.corner_small(),
                                    );
                                }
                                if i == self.shot {
                                    ui.painter().rect_stroke(
                                        r,
                                        t.corner_small(),
                                        Stroke::new(2.0, Color32::WHITE),
                                        egui::StrokeKind::Inside,
                                    );
                                    if center_now {
                                        resp.scroll_to_me(Some(Align::Center));
                                    }
                                } else if !resp.hovered() {
                                    ui.painter().rect_filled(
                                        r,
                                        t.corner_small(),
                                        Color32::from_black_alpha(90),
                                    );
                                }
                                let resp = if i < n_tr {
                                    resp.on_hover_text(&trailers[i].name)
                                } else {
                                    resp
                                };
                                if resp
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    pick = Some(i);
                                }
                            }
                        });
                    });
            });
            if arrow(ui, icons::CARET_RIGHT) && total > 0 {
                pick = Some((self.shot + 1) % total);
            }
        });
        if center_now {
            self.shot_centered = Some(self.shot);
        }
        if let Some(i) = pick {
            self.shot = i;
        }
    }
}

/// Text that underlines on hover, like Steam's store links.
fn link(ui: &mut Ui, t: &Theme, text: &str, size: f32, color: Color32) -> egui::Response {
    let r = ui.add(
        egui::Label::new(w::text(text, size, color))
            .sense(Sense::click())
            .selectable(false),
    );
    if r.hovered() {
        ui.painter().hline(
            r.rect.x_range(),
            r.rect.bottom() - 1.0,
            Stroke::new(1.0, t.text),
        );
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A tag on a game page: a pill you can click.
fn tag_chip(ui: &mut Ui, t: &Theme, name: &str) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), w::font(12.5), t.text_dim);
    let (r, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 10.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(
        r,
        CornerRadius::same(4),
        w::mix(t.raised, t.accent.gamma_multiply(0.35), hover),
    );
    let color = w::mix(t.text_dim, t.text, hover);
    ui.painter()
        .galley(r.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn fact(ui: &mut Ui, t: &Theme, label: &str, value: &str) {
    ui.label(w::text_bold(label.to_uppercase(), 10.5, t.text_faint));
    ui.add(egui::Label::new(w::text(value, 13.0, t.text)).truncate());
    ui.end_row();
}

fn thousands(n: u64) -> String {
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

/// One store card: header art, name, price.
fn card(ui: &mut Ui, t: &Theme, item: &Item, width: f32, owned: bool) -> egui::Response {
    let h = width * 215.0 / 460.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h + 64.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let lift = rect.translate(vec2(0.0, -3.0 * hover));
    let p = ui.painter();
    if hover > 0.0 {
        p.rect_filled(
            lift.translate(vec2(0.0, 10.0)),
            t.corner(),
            Color32::from_black_alpha((80.0 * hover) as u8),
        );
    }
    p.rect_filled(lift, t.corner(), w::mix(t.surface, t.surface_hover, hover));
    let img = Rect::from_min_size(lift.min, vec2(width, h));
    w::image_or_skeleton(
        ui,
        t,
        img,
        &item.header,
        CornerRadius {
            nw: t.radius,
            ne: t.radius,
            sw: 0,
            se: 0,
        },
    );
    if owned {
        in_library_badge(ui, t, img);
    }
    let body = Rect::from_min_max(
        pos2(lift.left() + 12.0, img.bottom() + 8.0),
        lift.max - vec2(12.0, 8.0),
    );
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    c.spacing_mut().item_spacing.y = 2.0;
    c.add(egui::Label::new(w::text_medium(&item.name, 14.0, t.text)).truncate());
    c.with_layout(Layout::right_to_left(Align::Center), |ui| {
        // Like Steam: games you own say so instead of showing a price.
        if owned {
            ui.label(w::text_semibold(t!("In library"), 13.0, t.play));
        } else {
            w::price_tag(
                ui,
                t,
                &item.price.label(),
                item.price.original_label().as_deref(),
                item.price.discount,
            );
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if item.windows {
                ui.label(w::text(icons::WINDOWS, 13.0, t.text_faint));
            }
        });
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "In library" ribbon on a store picture's top-left corner, like Steam's.
fn in_library_badge(ui: &Ui, t: &Theme, img: Rect) {
    let text = format!("{}  {}", icons::CHECK, t!("In library"));
    let galley = ui
        .painter()
        .layout_no_wrap(text, w::font_semibold(11.5), Color32::WHITE);
    let r = Rect::from_min_size(img.min + vec2(8.0, 8.0), galley.size() + vec2(14.0, 8.0));
    ui.painter()
        .rect_filled(r, CornerRadius::same(4), t.play.gamma_multiply(0.92));
    ui.painter()
        .galley(r.min + vec2(7.0, 4.0), galley, Color32::WHITE);
}

/// Lay out store HTML / Steam BBCode blocks.
pub fn blocks(ui: &mut Ui, t: &Theme, blocks: &[Block], width: f32) {
    let job = |runs: &[(String, bool)], color: Color32| {
        let mut job = LayoutJob::default();
        for (i, (text, bold)) in runs.iter().enumerate() {
            let sep = if i > 0 { " " } else { "" };
            let font = if *bold {
                w::font_semibold(14.0)
            } else {
                w::font(14.0)
            };
            job.append(
                &format!("{sep}{text}"),
                0.0,
                TextFormat {
                    font_id: font,
                    color,
                    line_height: Some(21.0),
                    ..Default::default()
                },
            );
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
                ui.add(
                    egui::Image::new(url.clone())
                        .max_width(width.min(860.0))
                        .corner_radius(t.corner_small()),
                );
                ui.add_space(6.0);
            }
            Block::Video(id) => {
                ui.add_space(6.0);
                let w_ = width.min(640.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w_, w_ * 0.5625), Sense::click());
                w::image_or_skeleton(
                    ui,
                    t,
                    r,
                    &format!("https://img.youtube.com/vi/{id}/hqdefault.jpg"),
                    t.corner_small(),
                );
                ui.painter()
                    .rect_filled(r, t.corner_small(), Color32::from_black_alpha(60));
                ui.painter()
                    .circle_filled(r.center(), 30.0, Color32::from_rgb(0xE5, 0x2D, 0x27));
                ui.painter().text(
                    r.center() + vec2(2.0, 0.0),
                    Align2::CENTER_CENTER,
                    icons::PLAY,
                    w::font(26.0),
                    Color32::WHITE,
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(t!("Watch on YouTube"))
                    .clicked()
                {
                    let _ = open::that_detached(format!("https://www.youtube.com/watch?v={id}"));
                }
                ui.add_space(6.0);
            }
        }
    }
}

impl Store {
    /// "Is this game relevant to you?" — what Steam users check before buying.
    fn relevance(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &Details) {
        let info = self.browse_info(id);
        let local = ctx.local.games.iter().find(|g| g.app_id == id);
        let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) || local.is_some();
        let top_seller = matches!(self.front_data(), Some(Ok((_, ref s))) if s.iter().any(|s| s.id == "top_sellers" && s.items.iter().any(|i| i.app_id == id)));
        let playing: Vec<_> = ctx
            .state
            .friend_list()
            .into_iter()
            .filter(|p| p.game_app_id == Some(id))
            .collect();

        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_semibold(
                t!("Is this game relevant to you?"),
                15.0,
                t.text,
            ));
            ui.add_space(6.0);
            let row = |ui: &mut Ui, icon: &str, color: Color32, text: String| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(w::text(icon, 15.0, color));
                    ui.label(w::text(text, 13.5, t.text_dim));
                });
            };
            if let Some(m) = local.and_then(|g| g.playtime_minutes).filter(|m| *m > 0) {
                row(
                    ui,
                    icons::INFO,
                    t.away,
                    t!("You've played {time}", time = ui::playtime(m)),
                );
            } else if owned {
                row(
                    ui,
                    icons::CHECK,
                    t.play,
                    t!("It's already in your library").to_string(),
                );
            }
            if let Some(r) = info.as_ref().and_then(|i| i.reviews.as_ref()) {
                let (icon, color) = if r.score >= 7 {
                    (icons::CHECK, t.play)
                } else if r.score <= 4 {
                    (icons::X, t.danger)
                } else {
                    (icons::INFO, t.away)
                };
                row(
                    ui,
                    icon,
                    color,
                    t!("User reviews: {label}", label = i18n::tr_dyn(&r.label)),
                );
            }
            if top_seller {
                row(
                    ui,
                    icons::CHECK,
                    t.play,
                    t!("In the Top Sellers").to_string(),
                );
            }
            if let Some(m) = d.metacritic.filter(|m| *m >= 75) {
                row(
                    ui,
                    icons::CHECK,
                    t.play,
                    t!("Metacritic score: {score}", score = m),
                );
            }
            if !playing.is_empty() {
                row(
                    ui,
                    icons::USERS,
                    t.in_game,
                    t!("Friends playing right now: {n}", n = playing.len()),
                );
                ui.horizontal_wrapped(|ui| {
                    for p in playing.iter().take(8) {
                        w::avatar(
                            ui,
                            t,
                            &model::art::avatar(p.avatar_hash.as_deref()),
                            30.0,
                            Some(t.in_game),
                        )
                        .on_hover_text(&p.name);
                    }
                });
            }
            if d.coming_soon {
                row(
                    ui,
                    icons::CALENDAR,
                    t.accent,
                    t!("Not released yet").to_string(),
                );
            }
        });
    }
}

fn review_color(t: &Theme, score: u32) -> Color32 {
    match score {
        7..=9 => Color32::from_rgb(0x66, 0xC0, 0xF4),
        5..=6 => Color32::from_rgb(0xB9, 0xA0, 0x74),
        _ => t.danger,
    }
}

/// Icon for a store "feature" category (Co-op, Trading Cards, ...).
fn feature_icon(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("single") || n.contains("одиноч") {
        icons::USER
    } else if n.contains("multi")
        || n.contains("co-op")
        || n.contains("pvp")
        || n.contains("мульти")
        || n.contains("кооп")
    {
        icons::USERS
    } else if n.contains("achiev") || n.contains("достиж") {
        icons::TROPHY
    } else if n.contains("card") || n.contains("карточ") {
        icons::TAG
    } else if n.contains("cloud") || n.contains("облак") {
        icons::DOWNLOAD
    } else if n.contains("controller") || n.contains("контрол") {
        icons::CONTROLLER
    } else if n.contains("workshop") || n.contains("мастерск") {
        icons::SLIDERS
    } else if n.contains("anti-cheat") || n.contains("vac") || n.contains("античит") {
        icons::SHIELD
    } else if n.contains("purchase") || n.contains("покуп") {
        icons::CART
    } else {
        icons::CHECK
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
        match self.safe(id) {
            Some(ok) => !ok,
            // Until we know, use the details call if it has already answered.
            None => self.details(id).is_some_and(|d| {
                d.content_descriptors
                    .iter()
                    .any(|x| self.hidden.contains(x))
            }),
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
                });
            });
        });
    }
}

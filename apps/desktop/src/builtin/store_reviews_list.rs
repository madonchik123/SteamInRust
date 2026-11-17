//! Store: what players wrote, laid out like Steam's game page: reviews by
//! your friends first, then the most helpful ones, with the newest in a
//! column beside them. Each review can be voted on ("Was this review
//! helpful?").

use super::{Load, Shared, Store};
use client::Command;
use egui::{Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, ReviewVote, SteamId, art};
use std::collections::{HashMap, HashSet};
use store::reviews::{Order, Review, ReviewPage};
use ui::{Theme, icons, widgets as w};

#[derive(Default)]
pub(super) struct ReviewList {
    /// Pages loaded so far, per (game, newest first).
    pages: HashMap<(AppId, bool), Vec<Shared<ReviewPage>>>,
    /// Reviews opened in full.
    expanded: HashSet<String>,
    /// Games whose friends' reviews were asked for.
    friends_asked: HashSet<AppId>,
    /// Authors whose review ids were asked for (to vote).
    ids_asked: HashSet<(AppId, SteamId)>,
}

/// Long reviews show this much until "Read more".
const SHORT: usize = 600;

/// One review, whoever it comes from.
struct Card<'a> {
    key: String,
    author: String,
    avatar: String,
    products: String,
    voted_up: bool,
    title: String,
    hours: String,
    posted: String,
    text: &'a str,
    /// Steam's id for voting, once known.
    review: Option<u64>,
    votes: String,
    friend: bool,
}

impl Store {
    fn page(&mut self, id: AppId, order: Order) -> Vec<Option<ReviewPage>> {
        let key = (id, order == Order::Recent);
        if !self.reviews_list.pages.contains_key(&key) {
            let api = self.api.clone();
            let first = self.fetch(async move { api.reviews(id, order, 1).await });
            self.reviews_list.pages.insert(key, vec![first]);
        }
        self.reviews_list.pages[&key]
            .iter()
            .map(|p| match &*p.lock().unwrap() {
                Load::Ready(r) => Some(r.clone()),
                Load::Failed(_) => Some(ReviewPage::default()),
                Load::Loading => None,
            })
            .collect()
    }

    fn more(&mut self, id: AppId, order: Order, next: u32) {
        let api = self.api.clone();
        let page = self.fetch(async move { api.reviews(id, order, next).await });
        self.reviews_list.pages.get_mut(&(id, order == Order::Recent)).expect("loaded").push(page);
    }

    /// Ask Steam for these authors' review ids (to vote), once.
    fn want_ids(&mut self, ctx: &Ctx, id: AppId, reviews: &[Review]) {
        let people: Vec<SteamId> = reviews.iter().map(|r| r.author_id).filter(|a| *a != 0 && self.reviews_list.ids_asked.insert((id, *a))).collect();
        if !people.is_empty() && ctx.state.steam_id != 0 {
            ctx.send(Command::LoadReviewsBy { app_id: id, people });
        }
    }

    pub(super) fn review_list(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        if ctx.state.steam_id != 0 && self.reviews_list.friends_asked.insert(id) {
            ctx.send(Command::LoadFriendReviews { app_id: id });
        }
        let helpful = self.page(id, Order::Helpful);
        let recent = self.page(id, Order::Recent);
        for p in helpful.iter().chain(recent.iter()).flatten() {
            self.want_ids(ctx, id, &p.reviews);
        }
        let full = ui.available_width();
        let right_w = (full * 0.34).clamp(260.0, 400.0);
        let left_w = full - right_w - 24.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(left_w);
                // Reviews by friends.
                let friends = ctx.state.friend_reviews.get(&id).and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
                if !friends.is_empty() {
                    w::section(ui, t, t!("Reviews by friends"), None);
                    for (who, r) in &friends {
                        let p = ctx.state.persona(*who);
                        let title = if r.voted_up { t!("Recommended by your friend") } else { t!("Not recommended by your friend") };
                        let card = Card {
                            key: r.id.to_string(),
                            author: p.name.clone(),
                            avatar: art::avatar(p.avatar_hash.as_deref()),
                            products: String::new(),
                            voted_up: r.voted_up,
                            title: title.to_string(),
                            hours: hours(r.playtime, r.playtime_at_review),
                            posted: if r.updated > 0 { ui::ago(r.updated) } else { String::new() },
                            text: &r.text,
                            review: Some(r.id),
                            votes: votes(r.votes_up as u64, r.votes_funny as u64),
                            friend: true,
                        };
                        self.card(ui, t, ctx, &card, false);
                    }
                    ui.add_space(12.0);
                }
                w::section(ui, t, t!("Most helpful reviews"), None);
                self.list(ui, t, ctx, id, &helpful, Order::Helpful, false);
            });
            ui.vertical(|ui| {
                ui.set_width(right_w);
                w::section(ui, t, t!("Recently posted"), None);
                self.list(ui, t, ctx, id, &recent, Order::Recent, true);
            });
        });
        ui.add_space(24.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn list(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, pages: &[Option<ReviewPage>], order: Order, compact: bool) {
        let mut seen = HashSet::new();
        for page in pages.iter().flatten() {
            // Steam's pages can overlap when new reviews arrive meanwhile.
            for r in page.reviews.iter().filter(|r| seen.insert(r.id.clone())) {
                let known = ctx.state.reviews_by.get(&(id, r.author_id));
                let card = Card {
                    key: r.id.clone(),
                    author: r.author.clone(),
                    avatar: r.avatar.clone(),
                    products: r.products.clone(),
                    voted_up: r.voted_up,
                    title: if r.voted_up { t!("Recommended").to_string() } else { t!("Not recommended").to_string() },
                    // Steam's own numbers (total and at review time) once known.
                    hours: known.map(|k| hours(k.playtime, k.playtime_at_review)).filter(|h| !h.is_empty()).unwrap_or_else(|| r.hours.clone()),
                    posted: r.posted.clone(),
                    text: &r.text,
                    review: known.map(|k| k.id),
                    votes: [r.helpful.as_str(), r.funny.as_str()].into_iter().filter(|s| !s.is_empty() && !s.starts_with("No one")).collect::<Vec<_>>().join("\n"),
                    friend: false,
                };
                self.card(ui, t, ctx, &card, compact);
            }
        }
        if pages.iter().any(|p| p.is_none()) {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(t.accent));
                ui.label(w::text(t!("Loading reviews…"), 13.0, t.text_dim));
            });
        } else if seen.is_empty() {
            ui.label(w::text(t!("No reviews in your language yet."), 13.0, t.text_faint));
        } else if pages.last().and_then(|p| p.as_ref()).is_some_and(|p| p.more) && w::button(ui, t, Some(icons::CARET_DOWN), t!("Show more reviews"), w::Kind::Secondary, w::Size::Sm).clicked() {
            self.more(id, order, pages.len() as u32 + 1);
        }
    }

    fn card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, c: &Card, compact: bool) {
        let (icon, color) = if c.voted_up { (icons::THUMBS_UP, t.play) } else { (icons::THUMBS_DOWN, t.danger) };
        let mut frame = w::card(t);
        if c.friend {
            frame = frame.fill(t.accent.gamma_multiply(0.10));
        }
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                // The author.
                let (av, _) = ui.allocate_exact_size(vec2(if compact { 28.0 } else { 40.0 }, if compact { 28.0 } else { 40.0 }), egui::Sense::hover());
                w::image_or_skeleton(ui, t, av, &c.avatar, t.corner_small());
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(w::text_semibold(&c.author, 14.0, t.text));
                        if !compact && !c.products.is_empty() {
                            ui.label(w::text(&c.products, 12.0, t.text_faint));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(w::text(icon, if compact { 15.0 } else { 18.0 }, color));
                        ui.label(w::text_semibold(&c.title, if compact { 12.5 } else { 14.0 }, color));
                        if !c.hours.is_empty() {
                            ui.label(w::text(&c.hours, 12.0, t.text_faint));
                        }
                    });
                });
            });
            if !c.posted.is_empty() {
                ui.label(w::text(&c.posted, 11.5, t.text_faint));
            }
            ui.add_space(4.0);
            let open = self.reviews_list.expanded.contains(&c.key);
            let limit = if compact { SHORT / 2 } else { SHORT };
            let long = c.text.chars().count() > limit;
            let text: String = if long && !open { c.text.chars().take(limit).collect::<String>() + "…" } else { c.text.to_string() };
            ui.add(egui::Label::new(w::text(text, 13.5, t.text_dim)).wrap());
            if long && ui.link(w::text(if open { t!("Show less") } else { t!("Read more") }, 12.5, t.accent)).clicked() {
                if open {
                    self.reviews_list.expanded.remove(&c.key);
                } else {
                    self.reviews_list.expanded.insert(c.key.clone());
                }
            }
            ui.add_space(6.0);
            ui.separator();
            // "Was this review helpful?" (once Steam told us the review's id).
            if let Some(review) = c.review.filter(|_| ctx.state.steam_id != 0) {
                let mine = ctx.state.review_votes.get(&review).copied();
                ui.label(w::text(t!("Was this review helpful?"), 12.0, t.text_faint));
                ui.horizontal(|ui| {
                    for (vote, icon, label) in [(ReviewVote::Yes, icons::THUMBS_UP, t!("Yes")), (ReviewVote::No, icons::THUMBS_DOWN, t!("No")), (ReviewVote::Funny, icons::SMILEY, t!("Funny"))] {
                        if w::chip(ui, t, &format!("{icon}  {label}"), mine == Some(vote)).clicked() {
                            ctx.send(Command::VoteReview { review, vote });
                        }
                    }
                });
            }
            if !c.votes.is_empty() {
                ui.add_space(2.0);
                ui.label(w::text(&c.votes, 11.5, t.text_faint));
            }
        });
        ui.add_space(8.0);
    }
}

fn hours(total: u32, at_review: u32) -> String {
    let h = |m: u32| format!("{:.1}", m as f64 / 60.0);
    if total == 0 {
        return String::new();
    }
    let mut s = t!("{h} hrs on record", h = h(total)).to_string();
    if at_review > 0 && at_review != total {
        s = format!("{s} ({})", t!("{h} hrs at review time", h = h(at_review)));
    }
    s
}

fn votes(up: u64, funny: u64) -> String {
    let mut v = Vec::new();
    if up > 0 {
        v.push(t!("{n} found it helpful", n = up).to_string());
    }
    if funny > 0 {
        v.push(t!("{n} found it funny", n = funny).to_string());
    }
    v.join("\n")
}

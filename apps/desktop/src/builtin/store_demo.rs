//! `--demo`: a made-up wishlist, cart and discovery queue (real store
//! games), so the store's own pages have something to show.

use client::Event;
use model::CartLine;

pub fn events(now: u64) -> Vec<Event> {
    const DAY: u64 = 86_400;
    let wishlist = [1245620, 1086940, 367520, 1145350, 504230, 1091500].iter().enumerate().map(|(i, id)| (*id, now - (i as u64 + 2) * 9 * DAY)).collect();
    let cart = vec![
        CartLine { line_item_id: 1, package_id: 66613, price: "$14.99".into(), gift_to: None, is_gift: false },
        CartLine { line_item_id: 2, package_id: 907539, price: "$59.99".into(), gift_to: None, is_gift: true },
    ];
    vec![Event::Wishlist(wishlist), Event::Cart(cart), Event::DiscoveryQueue(Ok(vec![1145350, 504230, 892970, 105600, 1794680, 1091500]))]
}

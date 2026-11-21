//! Bundles and packages: what's in one, what it costs, and what it costs you
//! when you already own part of it. From Steam's `IStoreBrowseService`.

use crate::{Error, Result, StoreClient};
use model::AppId;
use serde::Deserialize;

const API: &str = "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/";
const ASSETS: &str = "https://shared.akamai.steamstatic.com/store_item_assets/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OfferKind {
    Bundle,
    Package,
}

impl OfferKind {
    fn key(self) -> &'static str {
        match self {
            OfferKind::Bundle => "bundleid",
            OfferKind::Package => "packageid",
        }
    }
}

/// One thing in a bundle (a package) or in a package (a game).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OfferPart {
    /// 0 when this part is a game inside a package.
    pub package_id: u32,
    pub apps: Vec<AppId>,
    pub name: String,
    pub art: String,
    /// Its price alone; `None` if Steam didn't give one.
    pub cents: Option<u64>,
    pub price: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub kind: OfferKind,
    pub id: u32,
    pub name: String,
    pub art: String,
    pub parts: Vec<OfferPart>,
    /// What it costs; `None` when it isn't on sale.
    pub cents: Option<u64>,
    pub price: String,
    /// A bundle's price before its discount: what its parts add up to.
    pub before_bundle: Option<u64>,
    pub before_text: String,
    pub bundle_pct: u32,
    pub discount_pct: u32,
    /// Sold only whole, even to someone who owns part of it.
    pub as_set: bool,
}

/// What this offer means for someone owning some of it.
#[derive(Debug, Clone, PartialEq)]
pub enum YourPrice {
    /// You own none of it: the price shown.
    Full,
    OwnAll,
    /// "Complete your collection": only what's missing, about this much.
    Complete { missing: usize, cents: u64, text: String },
    /// You own some, but it's only sold whole.
    WholeSet { owned: usize },
    /// You own some, and only Steam's cart can say what the rest costs.
    InCart { missing: usize },
}

impl Offer {
    /// `owned` says whether you have a part.
    pub fn your_price(&self, owned: impl Fn(&OfferPart) -> bool) -> YourPrice {
        let have = self.parts.iter().filter(|p| owned(p)).count();
        if have == 0 {
            return YourPrice::Full;
        }
        if have == self.parts.len() {
            return YourPrice::OwnAll;
        }
        if self.kind == OfferKind::Package || self.as_set {
            return YourPrice::WholeSet { owned: have };
        }
        let missing = self.parts.len() - have;
        // Only when the parts add up to Steam's own total: during a sale they
        // may not, and then a sum here would be a made-up price.
        let total: Option<u64> = self.parts.iter().map(|p| p.cents).sum();
        if total.is_none() || total != self.before_bundle {
            return YourPrice::InCart { missing };
        }
        let keep = 100 - u64::from(self.bundle_pct.min(100));
        // Steam takes the discount off each item, rounding each one down.
        let cents: u64 = self.parts.iter().filter(|p| !owned(p)).filter_map(|p| p.cents).map(|c| c * keep / 100).sum();
        match format_like(&self.price, cents) {
            Some(text) => YourPrice::Complete { missing, cents, text },
            None => YourPrice::InCart { missing },
        }
    }
}

/// `cents` written the way Steam wrote `sample` ("$116.84", "104,88€",
/// "5328 руб", "¥16,404"): same symbol, separators and decimals.
pub fn format_like(sample: &str, cents: u64) -> Option<String> {
    let start = sample.find(|c: char| c.is_ascii_digit())?;
    let end = sample.rfind(|c: char| c.is_ascii_digit())? + 1;
    let (prefix, number, suffix) = (&sample[..start], &sample[start..end], &sample[end..]);
    let seps: Vec<(usize, char)> = number.char_indices().filter(|(_, c)| !c.is_ascii_digit()).collect();
    let decimal = seps.last().filter(|(i, c)| number.len() - i - c.len_utf8() == 2).map(|(_, c)| *c);
    let group = seps.iter().map(|(_, c)| *c).find(|c| Some(*c) != decimal);
    let whole = if decimal.is_some() { cents / 100 } else { (cents + 50) / 100 };
    let digits = whole.to_string();
    let mut body = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            body.extend(group);
        }
        body.push(d);
    }
    if let Some(d) = decimal {
        body.push(d);
        body.push_str(&format!("{:02}", cents % 100));
    }
    Some(format!("{prefix}{body}{suffix}"))
}

/// Another way to buy a game: a bundle or an edition.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferLink {
    pub kind: OfferKind,
    pub id: u32,
    pub name: String,
    pub price: String,
    pub discount_pct: u32,
}

impl StoreClient {
    async fn store_items(&self, ids: serde_json::Value, data: serde_json::Value) -> Result<Vec<RawItem>> {
        let input = serde_json::json!({
            "ids": ids,
            "context": { "language": self.language, "country_code": self.country.to_uppercase() },
            "data_request": data,
        });
        let raw: Raw = self.http.get(API).query(&[("input_json", input.to_string())]).send().await?.error_for_status()?.json().await?;
        Ok(raw.response.store_items)
    }

    /// A bundle's or package's page.
    pub async fn offer(&self, kind: OfferKind, id: u32) -> Result<Offer> {
        let items = self
            .store_items(
                serde_json::json!([{ (kind.key()): id }]),
                serde_json::json!({ "include_assets": true, "include_basic_info": true, "include_all_purchase_options": true, "include_included_items": true }),
            )
            .await?;
        let item = items.into_iter().next().filter(|i| !i.name.is_empty()).ok_or(Error::Unavailable)?;
        Ok(offer_of(kind, id, item))
    }

    /// The bundles and editions a game is sold in.
    pub async fn offers_for_app(&self, app: AppId) -> Result<Vec<OfferLink>> {
        let items = self.store_items(serde_json::json!([{ "appid": app }]), serde_json::json!({ "include_all_purchase_options": true })).await?;
        Ok(items.into_iter().next().map(|i| links_of(&i)).unwrap_or_default())
    }

    /// Names and art for bundles (cart lines can be bundles):
    /// (bundle id, name, art, first game in it).
    pub async fn bundle_names(&self, ids: &[u32]) -> Result<Vec<(u32, String, String, AppId)>> {
        let list: Vec<_> = ids.iter().map(|id| serde_json::json!({ "bundleid": id })).collect();
        let items = self.store_items(serde_json::Value::Array(list), serde_json::json!({ "include_assets": true })).await?;
        Ok(items
            .into_iter()
            .map(|i| {
                let first = i.included_appids.first().copied().unwrap_or(0);
                let art = art_of(&i).unwrap_or_else(|| model::art::header(first));
                (i.id, i.name, art, first)
            })
            .collect())
    }
}

fn cents(s: &Option<String>) -> Option<u64> {
    s.as_deref().and_then(|v| v.parse().ok())
}

fn art_of(i: &RawItem) -> Option<String> {
    let a = i.assets.as_ref()?;
    let file = a.main_capsule.as_deref().or(a.header.as_deref())?;
    Some(format!("{ASSETS}{}", a.asset_url_format.as_deref()?.replace("${FILENAME}", file)))
}

fn part_of(p: RawPart, in_bundle: bool) -> OfferPart {
    let apps = if !p.included_appids.is_empty() {
        p.included_appids
    } else if p.appid != 0 {
        vec![p.appid]
    } else if !in_bundle && p.id != 0 {
        vec![p.id]
    } else {
        Vec::new()
    };
    let o = p.best_purchase_option.unwrap_or_default();
    OfferPart {
        package_id: if in_bundle { p.id } else { 0 },
        art: apps.first().map(|a| model::art::header(*a)).unwrap_or_default(),
        apps,
        name: p.name,
        cents: cents(&o.final_price_in_cents),
        price: o.formatted_final_price,
    }
}

fn offer_of(kind: OfferKind, id: u32, i: RawItem) -> Offer {
    let art = art_of(&i).unwrap_or_else(|| model::art::header(i.included_appids.first().copied().unwrap_or(0)));
    let RawIncluded { included_apps, included_packages } = i.included_items;
    let parts: Vec<OfferPart> = if kind == OfferKind::Bundle && !included_packages.is_empty() {
        included_packages.into_iter().map(|p| part_of(p, true)).collect()
    } else {
        included_apps.into_iter().map(|p| part_of(p, false)).collect()
    };
    let o = i.best_purchase_option.unwrap_or_default();
    let before_text = o.formatted_price_before_bundle_discount.or(o.formatted_original_price).unwrap_or_default();
    Offer {
        kind,
        id,
        name: i.name,
        art,
        parts,
        cents: cents(&o.final_price_in_cents),
        price: o.formatted_final_price,
        before_bundle: cents(&o.price_before_bundle_discount),
        before_text,
        bundle_pct: o.bundle_discount_pct,
        discount_pct: [o.combined_discount_pct, o.discount_pct, o.bundle_discount_pct].into_iter().find(|d| *d > 0).unwrap_or(0),
        as_set: o.must_purchase_as_set,
    }
}

fn links_of(i: &RawItem) -> Vec<OfferLink> {
    i.purchase_options
        .iter()
        .filter_map(|o| {
            let (kind, id) = match (o.bundleid, o.packageid) {
                (Some(b), _) if b != 0 => (OfferKind::Bundle, b),
                (_, Some(p)) if p != 0 => (OfferKind::Package, p),
                _ => return None,
            };
            let discount_pct = [o.combined_discount_pct, o.discount_pct, o.bundle_discount_pct].into_iter().find(|d| *d > 0).unwrap_or(0);
            Some(OfferLink { kind, id, name: o.purchase_option_name.clone(), price: o.formatted_final_price.clone(), discount_pct })
        })
        .collect()
}

#[derive(Deserialize)]
struct Raw {
    response: RawResponse,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawResponse {
    store_items: Vec<RawItem>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawItem {
    id: u32,
    name: String,
    assets: Option<RawAssets>,
    best_purchase_option: Option<RawOption>,
    purchase_options: Vec<RawOption>,
    included_items: RawIncluded,
    included_appids: Vec<AppId>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawAssets {
    asset_url_format: Option<String>,
    main_capsule: Option<String>,
    header: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawIncluded {
    included_apps: Vec<RawPart>,
    included_packages: Vec<RawPart>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawPart {
    id: u32,
    appid: AppId,
    name: String,
    included_appids: Vec<AppId>,
    best_purchase_option: Option<RawOption>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawOption {
    packageid: Option<u32>,
    bundleid: Option<u32>,
    purchase_option_name: String,
    final_price_in_cents: Option<String>,
    formatted_final_price: String,
    formatted_original_price: Option<String>,
    price_before_bundle_discount: Option<String>,
    formatted_price_before_bundle_discount: Option<String>,
    bundle_discount_pct: u32,
    discount_pct: u32,
    combined_discount_pct: u32,
    must_purchase_as_set: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: u32, app: AppId, c: u64) -> OfferPart {
        OfferPart { package_id: id, apps: vec![app], cents: Some(c), price: format!("${}.{:02}", c / 100, c % 100), ..Default::default() }
    }

    fn bundle(parts: Vec<OfferPart>, pct: u32, as_set: bool) -> Offer {
        let before: u64 = parts.iter().filter_map(|p| p.cents).sum();
        let cents: u64 = parts.iter().filter_map(|p| p.cents).map(|c| c * (100 - pct as u64) / 100).sum();
        Offer {
            kind: OfferKind::Bundle,
            id: 1,
            name: "B".into(),
            art: String::new(),
            parts,
            cents: Some(cents),
            price: format!("${}.{:02}", cents / 100, cents % 100),
            before_bundle: Some(before),
            before_text: String::new(),
            bundle_pct: pct,
            discount_pct: pct,
            as_set,
        }
    }

    #[test]
    fn complete_your_collection_counts_only_whats_missing() {
        let b = bundle(vec![part(1, 10, 999), part(2, 20, 1999), part(3, 30, 499)], 10, false);
        let your = b.your_price(|p| p.package_id == 2);
        // 999 and 499, 10% off each, each rounded down: 899 + 449.
        assert_eq!(your, YourPrice::Complete { missing: 2, cents: 1348, text: "$13.48".into() });
    }

    #[test]
    fn owning_none_or_all() {
        let b = bundle(vec![part(1, 10, 999), part(2, 20, 1999)], 10, false);
        assert_eq!(b.your_price(|_| false), YourPrice::Full);
        assert_eq!(b.your_price(|_| true), YourPrice::OwnAll);
    }

    #[test]
    fn a_set_is_never_discounted_for_what_you_own() {
        let b = bundle(vec![part(1, 10, 999), part(2, 20, 1999)], 10, true);
        assert_eq!(b.your_price(|p| p.package_id == 1), YourPrice::WholeSet { owned: 1 });
    }

    #[test]
    fn no_made_up_price_when_the_parts_dont_add_up() {
        // During a sale the parts can differ from Steam's total.
        let mut b = bundle(vec![part(1, 10, 999), part(2, 20, 1999)], 10, false);
        b.before_bundle = Some(5000);
        assert_eq!(b.your_price(|p| p.package_id == 1), YourPrice::InCart { missing: 1 });
        let mut c = bundle(vec![part(1, 10, 999), part(2, 20, 1999)], 10, false);
        c.parts[1].cents = None;
        assert_eq!(c.your_price(|p| p.package_id == 1), YourPrice::InCart { missing: 1 });
    }

    #[test]
    fn prices_are_written_the_way_steam_wrote_them() {
        assert_eq!(format_like("$116.84", 1348).as_deref(), Some("$13.48"));
        assert_eq!(format_like("104,88€", 123456).as_deref(), Some("1234,56€"));
        assert_eq!(format_like("R$403,04", 5).as_deref(), Some("R$0,05"));
        assert_eq!(format_like("5328 руб", 120000).as_deref(), Some("1200 руб"));
        assert_eq!(format_like("¥16,404", 1234500).as_deref(), Some("¥12,345"));
        assert_eq!(format_like("$1,299.00", 123456).as_deref(), Some("$1,234.56"));
        assert_eq!(format_like("Free", 100), None);
    }

    #[test]
    fn reads_a_bundle_as_steam_sends_it() {
        let json = r#"{"response":{"store_items":[{"id":232,"name":"Valve Complete Pack","included_appids":[240,620],
            "assets":{"asset_url_format":"steam/bundles/232/x/${FILENAME}?t=1","main_capsule":"capsule_616x353.jpg"},
            "best_purchase_option":{"bundleid":232,"final_price_in_cents":"1348","formatted_final_price":"$13.48","bundle_discount_pct":10,
              "price_before_bundle_discount":"1498","formatted_price_before_bundle_discount":"$14.98","must_purchase_as_set":false,"combined_discount_pct":10},
            "included_items":{"included_apps":[{"appid":240,"id":240,"name":"CS"}],"included_packages":[
              {"id":37,"appid":240,"name":"Counter-Strike: Source","included_appids":[240],"best_purchase_option":{"packageid":37,"final_price_in_cents":"999","formatted_final_price":"$9.99"}},
              {"id":7877,"appid":620,"name":"Portal 2","included_appids":[620],"best_purchase_option":{"packageid":7877,"final_price_in_cents":"499","formatted_final_price":"$4.99"}}]}}]}}"#;
        let raw: Raw = serde_json::from_str(json).unwrap();
        let o = offer_of(OfferKind::Bundle, 232, raw.response.store_items.into_iter().next().unwrap());
        assert_eq!((o.name.as_str(), o.cents, o.before_bundle, o.bundle_pct, o.discount_pct), ("Valve Complete Pack", Some(1348), Some(1498), 10, 10));
        assert!(o.art.ends_with("steam/bundles/232/x/capsule_616x353.jpg?t=1"));
        assert_eq!(o.parts.len(), 2);
        assert_eq!((o.parts[1].package_id, o.parts[1].apps.as_slice(), o.parts[1].cents), (7877, &[620][..], Some(499)));
        assert_eq!(o.before_text, "$14.98");
    }

    #[test]
    fn a_package_lists_its_games() {
        let json = r#"{"id":7877,"name":"Portal 2","included_appids":[620],"best_purchase_option":{"packageid":7877,"final_price_in_cents":"999","formatted_final_price":"$9.99"},
            "included_items":{"included_apps":[{"appid":620,"id":620,"name":"Portal 2"}]}}"#;
        let o = offer_of(OfferKind::Package, 7877, serde_json::from_str(json).unwrap());
        assert_eq!((o.parts.len(), o.parts[0].package_id, o.parts[0].apps[0]), (1, 0, 620));
        assert_eq!(o.your_price(|_| true), YourPrice::OwnAll);
    }

    #[test]
    fn a_games_other_ways_to_buy_it() {
        let json = r#"{"id":620,"name":"Portal 2","purchase_options":[
            {"packageid":7877,"purchase_option_name":"Portal 2","formatted_final_price":"$9.99"},
            {"bundleid":234,"purchase_option_name":"Portal Bundle","formatted_final_price":"$14.98","bundle_discount_pct":25}]}"#;
        let links = links_of(&serde_json::from_str(json).unwrap());
        assert_eq!(links.len(), 2);
        assert_eq!((links[0].kind, links[0].id), (OfferKind::Package, 7877));
        assert_eq!((links[1].kind, links[1].id, links[1].discount_pct), (OfferKind::Bundle, 234, 25));
    }
}

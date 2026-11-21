//! Your account: security details and Steam Family through Steam's
//! services (what Steam's own client calls), and purchase history and
//! licenses from the store pages Steam's client shows for them (they have
//! no service). Everything here only reads.

use crate::market::Web;
use model::{AccountSecurity, Family, FamilyAction, FamilyInvite, FamilyMember, FamilyOutcome, FamilyRole, FamilyStatus, GuardDevice, License, Purchase, PurchasePage, SharedApp, SteamId};
use net::Connection;
use proto::webui::*;
use proto::*;

const STORE: &str = "https://store.steampowered.com";

/// Steam Guard, the mobile authenticator, phone and credential change dates.
pub async fn security(conn: &Connection) -> Result<AccountSecurity, String> {
    let guard: CCredentialsGetSteamGuardDetailsResponse =
        conn.call("Credentials.GetSteamGuardDetails#1", &CCredentialsGetSteamGuardDetailsRequest::default()).await.map_err(|e| e.to_string())?;
    let changes: Option<CCredentialsLastCredentialChangeTimeResponse> = conn
        .call("Credentials.GetCredentialChangeTimeDetails#1", &CCredentialsLastCredentialChangeTimeRequest { user_changes_only: Some(false) })
        .await
        .map_err(|e| tracing::debug!("credential changes: {e}"))
        .ok();
    let time = |t: Option<u32>| t.filter(|t| *t > 0).map(u64::from);
    Ok(AccountSecurity {
        steam_guard: guard.is_steamguard_enabled(),
        steam_guard_since: time(guard.timestamp_steamguard_enabled),
        authenticator: guard.is_twofactor_enabled(),
        authenticator_since: time(guard.timestamp_twofactor_enabled),
        phone_verified: guard.is_phone_verified(),
        password_changed: changes.as_ref().and_then(|c| time(c.timestamp_last_password_change)),
        email_changed: changes.as_ref().and_then(|c| time(c.timestamp_last_email_change)),
        password_reset: changes.as_ref().and_then(|c| time(c.timestamp_last_password_reset)),
        devices: guard
            .session_data
            .iter()
            .map(|s| GuardDevice {
                name: s.machine_name_userchosen().to_string(),
                since: time(s.timestamp_machine_steamguard_enabled),
                ip: s.public_ip_address().to_string(),
            })
            .collect(),
    })
}

/// Your Steam Family, its members and the games shared in it.
pub async fn family(conn: &Connection, me: SteamId, language: &str) -> Result<FamilyStatus, String> {
    let r: CFamilyGroupsGetFamilyGroupForUserResponse = conn
        .call("FamilyGroups.GetFamilyGroupForUser#1", &CFamilyGroupsGetFamilyGroupForUserRequest { steamid: Some(me), include_family_group_response: Some(true) })
        .await
        .map_err(|e| e.to_string())?;
    let mut status = FamilyStatus {
        family: None,
        role: FamilyRole::from_steam(r.role() as i32),
        cooldown: r.cooldown_seconds_remaining(),
        invites: r.pending_group_invites.iter().map(invite_for_you).collect(),
    };
    if r.is_not_member_of_any_group() || r.family_groupid() == 0 {
        return Ok(status);
    }
    let id = r.family_groupid();
    let g = match r.family_group {
        Some(g) => g,
        None => conn
            .call::<_, CFamilyGroupsGetFamilyGroupResponse>("FamilyGroups.GetFamilyGroup#1", &CFamilyGroupsGetFamilyGroupRequest { family_groupid: Some(id), send_running_apps: Some(false) })
            .await
            .map_err(|e| e.to_string())?,
    };
    let shared = conn
        .call::<_, CFamilyGroupsGetSharedLibraryAppsResponse>(
            "FamilyGroups.GetSharedLibraryApps#1",
            &CFamilyGroupsGetSharedLibraryAppsRequest {
                family_groupid: Some(id),
                include_own: Some(true),
                include_excluded: Some(true),
                language: Some(language.to_string()),
                max_apps: None,
                include_non_games: Some(false),
                steamid: Some(me),
            },
        )
        .await
        .map(|s| s.apps)
        .unwrap_or_else(|e| {
            tracing::warn!("family library: {e}");
            Vec::new()
        });
    let mut shared: Vec<SharedApp> = shared
        .into_iter()
        .map(|a| SharedApp { app_id: a.appid(), name: a.name().to_string(), owners: a.owner_steamids.clone(), excluded: a.exclude_reason() != 0 })
        .collect();
    shared.sort_by_key(|a| a.name.to_lowercase());
    status.family = Some(Family {
        id,
        name: g.name().to_string(),
        members: g
            .members
            .iter()
            .map(|m| FamilyMember { steam_id: m.steamid(), role: FamilyRole::from_steam(m.role()), joined: m.time_joined().into(), cooldown: m.cooldown_seconds_remaining() })
            .collect(),
        invites: g.pending_invites.iter().map(|i| (i.steamid(), FamilyRole::from_steam(i.role()))).collect(),
        free_spots: g.free_spots(),
        country: g.country().to_string(),
        slot_cooldown: g.slot_cooldown_remaining_seconds(),
        shared,
        former: g.former_members.iter().map(|m| m.steamid()).collect(),
        slot_overrides: g.slot_cooldown_overrides(),
    });
    Ok(status)
}

fn invite_for_you(i: &FamilyGroupPendingInviteForUser) -> FamilyInvite {
    FamilyInvite { family: i.family_groupid(), inviter: i.inviter_steamid(), role: FamilyRole::from_steam(i.role()), invite_id: i.invite_id(), awaiting_confirmation: i.awaiting_2fa() }
}

/// The one FamilyGroups call for an action. Kept apart from sending so
/// what goes to Steam can be tested without touching an account.
#[derive(Debug, PartialEq)]
enum FamilyCall {
    Invite(CFamilyGroupsInviteToFamilyGroupRequest),
    Cancel(CFamilyGroupsCancelFamilyGroupInviteRequest),
    Join(CFamilyGroupsJoinFamilyGroupRequest),
    ConfirmJoin(CFamilyGroupsConfirmJoinFamilyGroupRequest),
    Remove(CFamilyGroupsRemoveFromFamilyGroupRequest),
}

/// Declining is cancelling the invite to yourself and leaving is removing
/// yourself: that's how Steam's own family page does both.
fn family_call(action: &FamilyAction, me: SteamId) -> Result<FamilyCall, String> {
    let (family, target) = match action {
        FamilyAction::Invite { family, steam_id, .. } | FamilyAction::CancelInvite { family, steam_id } | FamilyAction::Remove { family, steam_id } => (*family, *steam_id),
        FamilyAction::Accept { family } | FamilyAction::ConfirmJoin { family, .. } | FamilyAction::Decline { family } | FamilyAction::Leave { family } => (*family, me),
    };
    if family == 0 || target == 0 {
        return Err("That family or account isn't valid".into());
    }
    let family_groupid = Some(family);
    Ok(match action {
        FamilyAction::Invite { role, .. } => {
            let role = role.to_steam().ok_or("Choose Adult or Child")?;
            FamilyCall::Invite(CFamilyGroupsInviteToFamilyGroupRequest { family_groupid, receiver_steamid: Some(target), receiver_role: Some(role) })
        }
        FamilyAction::CancelInvite { .. } | FamilyAction::Decline { .. } => FamilyCall::Cancel(CFamilyGroupsCancelFamilyGroupInviteRequest { family_groupid, steamid_to_cancel: Some(target) }),
        FamilyAction::Accept { .. } => FamilyCall::Join(CFamilyGroupsJoinFamilyGroupRequest { family_groupid, nonce: None }),
        FamilyAction::ConfirmJoin { invite_id, code, .. } => FamilyCall::ConfirmJoin(CFamilyGroupsConfirmJoinFamilyGroupRequest { family_groupid, invite_id: Some(*invite_id), nonce: Some(*code) }),
        FamilyAction::Remove { .. } | FamilyAction::Leave { .. } => FamilyCall::Remove(CFamilyGroupsRemoveFromFamilyGroupRequest { family_groupid, steamid_to_remove: Some(target) }),
    })
}

/// Steam's `two_factor_method`: 0 none, 1 Steam Mobile app, 2 email.
fn family_outcome(two_factor: i32, already_accepted: bool) -> FamilyOutcome {
    match (already_accepted, two_factor) {
        (true, _) => FamilyOutcome::AlreadyAccepted,
        (_, 0) => FamilyOutcome::Done,
        (_, 1) => FamilyOutcome::ConfirmOnPhone,
        (_, 2) => FamilyOutcome::ConfirmByEmail,
        _ => FamilyOutcome::ConfirmOther,
    }
}

/// Do something to a family. This changes the real account: the app only
/// calls it after the person confirmed in its dialog.
pub async fn family_action(conn: &Connection, me: SteamId, action: &FamilyAction) -> Result<FamilyOutcome, String> {
    let err = |e: net::Error| e.to_string();
    match family_call(action, me)? {
        FamilyCall::Invite(req) => {
            let r: CFamilyGroupsInviteToFamilyGroupResponse = conn.call("FamilyGroups.InviteToFamilyGroup#1", &req).await.map_err(err)?;
            Ok(family_outcome(r.two_factor_method(), false))
        }
        FamilyCall::Cancel(req) => {
            conn.call::<_, CFamilyGroupsCancelFamilyGroupInviteResponse>("FamilyGroups.CancelFamilyGroupInvite#1", &req).await.map_err(err)?;
            Ok(FamilyOutcome::Done)
        }
        FamilyCall::Join(req) => {
            let r: CFamilyGroupsJoinFamilyGroupResponse = conn.call("FamilyGroups.JoinFamilyGroup#1", &req).await.map_err(err)?;
            Ok(family_outcome(r.two_factor_method(), r.invite_already_accepted()))
        }
        FamilyCall::ConfirmJoin(req) => {
            conn.call::<_, CFamilyGroupsConfirmJoinFamilyGroupResponse>("FamilyGroups.ConfirmJoinFamilyGroup#1", &req).await.map_err(err)?;
            Ok(FamilyOutcome::Done)
        }
        FamilyCall::Remove(req) => {
            conn.call::<_, CFamilyGroupsRemoveFromFamilyGroupResponse>("FamilyGroups.RemoveFromFamilyGroup#1", &req).await.map_err(err)?;
            Ok(FamilyOutcome::Done)
        }
    }
}

/// A signed-in store page (`query` already has its `?`).
async fn store_page(web: &Web, path: &str, query: &str) -> Result<String, String> {
    let url = store_page_url(path, query)?;
    let body = crate::market::page(web, &url).await?;
    if body.contains("g_steamID = false;") {
        return Err("Steam didn't accept the web sign-in".into());
    }
    Ok(body)
}

fn store_page_url(path: &str, query: &str) -> Result<String, String> {
    if !path.starts_with('/') || path.contains('?') || path.contains('#') {
        return Err("invalid Steam store path".into());
    }
    if !query.is_empty() && (!query.starts_with('?') || query.contains('#')) {
        return Err("invalid Steam store query".into());
    }
    let parsed = crate::market::validate_steam_url(&format!("{STORE}{path}{query}"), "store.steampowered.com")?;
    Ok(parsed.into())
}

fn history_more_url(language: &str) -> String {
    format!("{STORE}/account/AjaxLoadMoreHistory/?l={}", crate::market::encode(language))
}

/// The inside of the first `<td>` whose class starts with `class`.
fn cell<'a>(row: &'a str, class: &str) -> Option<&'a str> {
    let at = row.find(&format!("class=\"{class}"))?;
    let open = row[at..].find('>')? + at + 1;
    let close = row[open..].find("</td>")? + open;
    Some(&row[open..close])
}

/// Visible text: tags removed, entities decoded (™, ®, numeric ones too).
fn text(html: &str) -> String {
    let s = crate::support::clean(html).replace("&trade;", "™").replace("&reg;", "®").replace("&copy;", "©");
    if !s.contains("&#") {
        return s;
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(i) = rest.find("&#") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let end = after.find(';').filter(|e| *e <= 7);
        let code = end.and_then(|e| if let Some(h) = after[..e].strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok() } else { after[..e].parse().ok() });
        match (end, code.and_then(char::from_u32)) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            _ => {
                out.push_str("&#");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of each `<div>` inside `html`, in order.
fn div_texts(html: &str) -> Vec<(bool, String)> {
    html.split("<div")
        .skip(1)
        .filter_map(|d| {
            let payment = d.starts_with(" class=\"wth_payment");
            let open = d.find('>')? + 1;
            let text = text(&d[open..d.find("</div>").unwrap_or(d.len())]);
            (!text.is_empty()).then_some((payment, text))
        })
        .collect()
}

/// Rows of the purchase history table (the page or a "load more" chunk).
pub fn parse_purchases(html: &str) -> Vec<Purchase> {
    html.split("class=\"wallet_table_row")
        .skip(1)
        .map(|row| {
            let row = &row[..row.find("</tr>").unwrap_or(row.len())];
            let field = |class: &str| cell(row, class).map(text).unwrap_or_default();
            let kind = cell(row, "wht_type").map(div_texts).unwrap_or_default();
            // Items are divs, or plain text (Market, conversions, wallet credit).
            let items = match cell(row, "wht_items") {
                Some(c) if c.contains("<div") => div_texts(c).into_iter().map(|(_, t)| t).collect(),
                Some(c) => Some(text(c)).filter(|t| !t.is_empty()).into_iter().collect(),
                None => Vec::new(),
            };
            Purchase {
                date: field("wht_date"),
                items,
                kind: kind.iter().find(|(p, _)| !p).map(|(_, t)| t.clone()).unwrap_or_default(),
                payment: kind.iter().find(|(p, _)| *p).map(|(_, t)| t.clone()).unwrap_or_default(),
                total: field("wht_total"),
                wallet_change: field("wht_wallet_change"),
                wallet_balance: field("wht_wallet_balance"),
                refunded: row.contains("wht_refunded") || row.contains("wht_item_refunded"),
            }
        })
        .collect()
}

/// `var g_historyCursor = {...};` from the history page (None when it ends).
fn history_cursor(body: &str) -> Option<String> {
    let start = body.find("var g_historyCursor = ")? + "var g_historyCursor = ".len();
    let end = body[start..].find(";\n").or_else(|| body[start..].find(';'))? + start;
    let json = body[start..end].trim();
    (json.starts_with('{')).then(|| json.to_string())
}

/// The first page of your purchase history, or the next one after `cursor`
/// (what Steam's "Load more transactions" button asks for).
pub async fn purchases(web: &Web, language: &str, cursor: Option<String>) -> Result<PurchasePage, String> {
    let Some(cursor) = cursor else {
        let query = format!("?l={}", crate::market::encode(language));
        let body = store_page(web, "/account/history/", &query).await?;
        return Ok(PurchasePage { rows: parse_purchases(&body), more: history_cursor(&body) });
    };
    let fields: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&cursor).map_err(|e| e.to_string())?;
    let mut form: Vec<(String, String)> = fields
        .iter()
        .map(|(k, v)| (format!("cursor[{k}]"), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
        .collect();
    form.push(("sessionid".into(), web.session_id.clone()));
    let r: serde_json::Value = web
        .http
        .post(history_more_url(language))
        .form(&form)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let rows = parse_purchases(r["html"].as_str().unwrap_or_default());
    let more = r.get("cursor").filter(|c| c.is_object()).map(|c| c.to_string());
    Ok(PurchasePage { rows, more })
}

/// Rows of the licenses table, and the link to the next page.
pub fn parse_licenses(body: &str) -> (Vec<License>, Option<String>) {
    let table = &body[body.find("class=\"account_table\"").unwrap_or(0)..];
    let rows = table
        .split("<tr")
        .skip(2)
        .filter_map(|row| {
            let row = &row[..row.find("</tr>").unwrap_or(row.len())];
            let mut cells = row.split("<td").skip(1).map(|c| {
                let open = c.find('>').map(|i| i + 1).unwrap_or(0);
                &c[open..c.find("</td>").unwrap_or(c.len())]
            });
            let (date, item, method) = (cells.next()?, cells.next()?, cells.next()?);
            // A "Remove" link (free licenses) sits before the name.
            let package_id = item.split("RemoveFreeLicense(").nth(1).and_then(|s| s.split(',').next()).and_then(|s| s.trim().parse().ok());
            let name = item.rsplit("</div>").next().unwrap_or(item);
            Some(License { date: text(date), name: text(name), method: text(method), package_id })
        })
        .collect();
    let next = body
        .split("class=\"license_paginator_next\" href=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .filter(|h| h.starts_with('?') && !h.contains('#'))
        .map(|h| h.replace("&amp;", "&"));
    (rows, next)
}

/// Every license on your account (Steam shows them 100 at a time).
pub async fn licenses(web: &Web, language: &str) -> Result<Vec<License>, String> {
    let mut all = Vec::new();
    let lang = crate::market::encode(language);
    let mut query = format!("?l={lang}");
    for _ in 0..50 {
        let body = store_page(web, "/account/licenses/", &query).await?;
        let (rows, next) = parse_licenses(&body);
        all.extend(rows);
        match next {
            Some(next) => query = format!("{next}&l={lang}"),
            None => break,
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchase_rows() {
        let html = r#"<tr class="wallet_table_row wallet_table_row_amt_change" onclick="x">
		<td class="wht_date">14 Oct, 2026</td>
		<td data-tooltip-text="Click" class="wht_items "">
			<div style="clear: both">PlateUp!</div></td>
		<td class="wht_type "><div>Purchase</div><div class="wth_payment">Wallet</div></td>
		<td class="wht_base_price"></td>
		<td class="wht_total ">520₸</td>
		<td class="wht_wallet_change wallet_column" data-tooltip-html="&lt;div&gt;">-520₸</td>
		<td class="wht_wallet_balance wallet_column" data-tooltip-html="&lt;b&gt;"> 522,58₸</td>
		</tr>
		<tr class="wallet_table_row ">
		<td class="wht_date">7 Oct, 2026</td>
		<td class="wht_items wht_item_refunded"><div>Dota 2</div><div class="wth_payment">2 Trove of Terror</div></td>
		<td class="wht_type wht_refunded"><div>Refund</div><div class="wth_payment">Wallet Credit</div></td>
		<td class="wht_total wht_refunded">1 300₸</td></tr>
		<script>var g_historyCursor = {"wallet_txnid":"764","timestamp_newest":1718964004,"balance":"29178","currency":37};
"#;
        let rows = parse_purchases(html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "14 Oct, 2026");
        assert_eq!(rows[0].items, vec!["PlateUp!"]);
        assert_eq!((rows[0].kind.as_str(), rows[0].payment.as_str()), ("Purchase", "Wallet"));
        assert_eq!((rows[0].total.as_str(), rows[0].wallet_change.as_str(), rows[0].wallet_balance.as_str()), ("520₸", "-520₸", "522,58₸"));
        assert!(!rows[0].refunded && rows[1].refunded);
        assert_eq!(rows[1].items, vec!["Dota 2", "2 Trove of Terror"]);
        let market = parse_purchases(r#"<tr class="wallet_table_row "><td class="wht_items "">
            Steam Community Market  </td><td class="wht_type "><div>Market Transaction</div></td></tr>"#);
        assert_eq!(market[0].items, vec!["Steam Community Market"]);
        assert_eq!(text("NieR&trade; &#8212; &#x41;"), "NieR™ — A");
        assert_eq!(history_cursor(html).as_deref(), Some(r#"{"wallet_txnid":"764","timestamp_newest":1718964004,"balance":"29178","currency":37}"#));
    }

    #[test]
    fn license_rows() {
        let html = r#"<a class="license_paginator_next" href="?continuationToken=17:37&offset=100">Next</a>
		<table class="account_table"><tr><th class="license_date_col">Date</th><th>Item</th><th>Acquisition Method</th></tr>
		<tr role="button"><td  class="license_date_col">14 Oct, 2026</td><td>
			PlateUp!</td><td class="license_acquisition_col">Steam Store</td></tr>
		<tr role="button"><td  class="license_date_col">9 Oct, 2026</td><td>
			<div class="free_license_remove_link"><a href="javascript:RemoveFreeLicense( 1335331, 'TXk' );">Remove</a></div>
			My Auto Clicker</td><td class="license_acquisition_col">Complimentary</td></tr></table>"#;
        let (rows, next) = parse_licenses(html);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].name.as_str(), rows[0].method.as_str(), rows[0].package_id), ("PlateUp!", "Steam Store", None));
        assert_eq!((rows[1].name.as_str(), rows[1].package_id), ("My Auto Clicker", Some(1335331)));
        assert_eq!(next.as_deref(), Some("?continuationToken=17:37&offset=100"));
    }

    #[test]
    fn store_urls_reject_traversal_and_encode_language() {
        assert!(store_page_url("/account/%2e%2e/admin/", "?l=english").is_err());
        assert!(store_page_url(".evil.tld/account/", "?l=english").is_err());
        let hist = history_more_url("english&evil=1");
        assert!(!hist.contains("&evil=1"), "language must be percent-encoded: {hist}");
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;
    use model::FamilyAction as A;
    use prost::Message;

    const ME: SteamId = 76561197960265729;
    const THEM: SteamId = 76561197960265829;

    #[test]
    fn invite_sends_the_person_and_role() {
        for (role, n) in [(FamilyRole::Adult, 1), (FamilyRole::Child, 2)] {
            let call = family_call(&A::Invite { family: 7, steam_id: THEM, role }, ME).unwrap();
            assert_eq!(call, FamilyCall::Invite(CFamilyGroupsInviteToFamilyGroupRequest { family_groupid: Some(7), receiver_steamid: Some(THEM), receiver_role: Some(n) }));
        }
        assert!(family_call(&A::Invite { family: 7, steam_id: THEM, role: FamilyRole::Unknown }, ME).is_err());
    }

    #[test]
    fn decline_cancels_your_own_invite_and_cancel_theirs() {
        let cancel = |id| FamilyCall::Cancel(CFamilyGroupsCancelFamilyGroupInviteRequest { family_groupid: Some(7), steamid_to_cancel: Some(id) });
        assert_eq!(family_call(&A::Decline { family: 7 }, ME).unwrap(), cancel(ME));
        assert_eq!(family_call(&A::CancelInvite { family: 7, steam_id: THEM }, ME).unwrap(), cancel(THEM));
    }

    #[test]
    fn leave_removes_you_and_remove_removes_them() {
        let remove = |id| FamilyCall::Remove(CFamilyGroupsRemoveFromFamilyGroupRequest { family_groupid: Some(7), steamid_to_remove: Some(id) });
        assert_eq!(family_call(&A::Leave { family: 7 }, ME).unwrap(), remove(ME));
        assert_eq!(family_call(&A::Remove { family: 7, steam_id: THEM }, ME).unwrap(), remove(THEM));
    }

    #[test]
    fn accept_and_the_emailed_code() {
        assert_eq!(family_call(&A::Accept { family: 7 }, ME).unwrap(), FamilyCall::Join(CFamilyGroupsJoinFamilyGroupRequest { family_groupid: Some(7), nonce: None }));
        assert_eq!(
            family_call(&A::ConfirmJoin { family: 7, invite_id: 9, code: 123456 }, ME).unwrap(),
            FamilyCall::ConfirmJoin(CFamilyGroupsConfirmJoinFamilyGroupRequest { family_groupid: Some(7), invite_id: Some(9), nonce: Some(123456) })
        );
    }

    #[test]
    fn refuses_empty_ids() {
        assert!(family_call(&A::Remove { family: 0, steam_id: THEM }, ME).is_err());
        assert!(family_call(&A::Remove { family: 7, steam_id: 0 }, ME).is_err());
        assert!(family_call(&A::Leave { family: 7 }, 0).is_err());
    }

    #[test]
    fn reads_how_steam_wants_it_confirmed() {
        assert_eq!(family_outcome(0, false), FamilyOutcome::Done);
        assert_eq!(family_outcome(1, false), FamilyOutcome::ConfirmOnPhone);
        assert_eq!(family_outcome(2, false), FamilyOutcome::ConfirmByEmail);
        assert_eq!(family_outcome(9, false), FamilyOutcome::ConfirmOther);
        assert_eq!(family_outcome(0, true), FamilyOutcome::AlreadyAccepted);
        // A join reply as it comes off the wire.
        let bytes = CFamilyGroupsJoinFamilyGroupResponse { two_factor_method: Some(2), cooldown_seconds_remaining: Some(86_400), ..Default::default() }.encode_to_vec();
        let r = CFamilyGroupsJoinFamilyGroupResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(family_outcome(r.two_factor_method(), r.invite_already_accepted()), FamilyOutcome::ConfirmByEmail);
    }

    #[test]
    fn reads_an_invite_to_you() {
        let wire = FamilyGroupPendingInviteForUser { family_groupid: Some(7), role: Some(2), inviter_steamid: Some(THEM), awaiting_2fa: Some(true), invite_id: Some(9) }.encode_to_vec();
        let i = invite_for_you(&FamilyGroupPendingInviteForUser::decode(wire.as_slice()).unwrap());
        assert_eq!(i, FamilyInvite { family: 7, inviter: THEM, role: FamilyRole::Child, invite_id: 9, awaiting_confirmation: true });
    }

    fn status(role: FamilyRole, free: u32, slot: u32, overrides: u32) -> FamilyStatus {
        let member = |steam_id, role| FamilyMember { steam_id, role, ..Default::default() };
        FamilyStatus {
            role,
            family: Some(Family { id: 7, members: vec![member(ME, role), member(THEM, FamilyRole::Child)], invites: vec![(5, FamilyRole::Adult)], free_spots: free, slot_cooldown: slot, slot_overrides: overrides, former: vec![6], ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn who_can_be_invited() {
        use model::InviteBlock as B;
        let open = status(FamilyRole::Adult, 2, 0, 0);
        assert_eq!(open.invite_block(), None);
        assert!(open.can_invite(99) && !open.can_invite(THEM) && !open.can_invite(5) && !open.can_invite(0));
        // A slot cooldown: only former members.
        let cooling = status(FamilyRole::Adult, 2, 1000, 0);
        assert_eq!(cooling.invite_block(), Some(B::SlotCooldown(1000)));
        assert!(cooling.can_invite(6) && !cooling.can_invite(99));
        assert_eq!(status(FamilyRole::Adult, 2, 1000, 1).invite_block(), None);
        let full = status(FamilyRole::Adult, 0, 0, 0);
        assert_eq!(full.invite_block(), Some(B::Full));
        assert!(!full.can_invite(6));
        let child = status(FamilyRole::Child, 2, 0, 0);
        assert_eq!(child.invite_block(), Some(B::NotAdult));
        assert!(!child.can_invite(99));
    }

    #[test]
    fn who_can_be_removed() {
        let adult = status(FamilyRole::Adult, 2, 0, 0);
        assert!(adult.can_remove(THEM) && adult.can_remove(ME) && !adult.can_remove(99));
        // Children can't remove anyone, or leave.
        let child = status(FamilyRole::Child, 2, 0, 0);
        assert!(!child.can_remove(THEM) && !child.can_remove(ME));
        // The last member deletes the family instead (on Steam's site).
        let mut alone = status(FamilyRole::Adult, 2, 0, 0);
        alone.family.as_mut().unwrap().members.truncate(1);
        assert!(!alone.can_remove(ME));
    }
}

//! ISteamInventory: the player's items for this game and its item
//! definitions, from Steam's Inventory service through SteamInRust. Drops,
//! promo items, consuming, crafting (ExchangeItems), stack moves and
//! property updates go to Steam when the game asks for them.
//!
//! Not supported: buying items (StartPurchase, prices) needs Steam's
//! overlay checkout, and DeserializeResult can't check Valve's signature on
//! another player's result, so it reports k_EResultFail.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, put, text};
use crate::protocol::{InvItem, InventoryOp, PropValue, Reply, Request};
use crate::state::{Bytes, lock, now};
use crate::vt::Fp;
use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{LazyLock, Mutex, MutexGuard};

const RESULT_READY: i32 = 4700;
const FULL_UPDATE: i32 = 4701;
const DEFINITION_UPDATE: i32 = 4702;
const ELIGIBLE_PROMO: i32 = 4703;
const START_PURCHASE: i32 = 4704;
const REQUEST_PRICES: i32 = 4705;

const OK: i32 = 1;
const FAIL: i32 = 2;
const PENDING: i32 = 22;

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "GetResultStatus" => f!(status),
        "GetResultItems" => f!(result_items),
        "GetResultItemProperty" => f!(result_property),
        "GetResultTimestamp" => f!(timestamp),
        "CheckResultSteamID" => f!(check_owner),
        "DestroyResult" => f!(destroy),
        "GetAllItems" => f!(all_items),
        "GetItemsByID" => f!(items_by_id),
        "SerializeResult" => f!(serialize),
        "DeserializeResult" => f!(deserialize),
        "GenerateItems" => f!(generate),
        "GrantPromoItems" => f!(grant_promo),
        "AddPromoItem" => f!(add_promo),
        "AddPromoItems" => f!(add_promos),
        "ConsumeItem" => f!(consume),
        "ExchangeItems" => f!(exchange),
        "TransferItemQuantity" => f!(transfer),
        "TriggerItemDrop" => f!(drop),
        "LoadItemDefinitions" => f!(load_defs),
        "GetItemDefinitionIDs" => f!(def_ids),
        "GetItemDefinitionProperty" => f!(def_property),
        "RequestEligiblePromoItemDefinitionsIDs" => f!(request_promo),
        "GetEligiblePromoItemDefinitionIDs" => f!(promo_ids),
        "StartPurchase" => f!(start_purchase),
        "RequestPrices" => f!(request_prices),
        "StartUpdateProperties" => f!(start_update),
        "RemoveProperty" => f!(remove_property),
        "SetPropertyString" => f!(set_string),
        "SetPropertyBool" => f!(set_bool),
        "SetPropertyInt64" => f!(set_int),
        "SetPropertyFloat" => f!(set_float),
        "SubmitUpdateProperties" => f!(submit_update),
        _ => None,
    }
}

struct Res {
    status: i32,
    items: Vec<InvItem>,
    ticket: Vec<u8>,
    owner: u64,
    time: u32,
}

#[derive(Default)]
struct Inv {
    next: i32,
    results: HashMap<i32, Res>,
    defs: Option<Vec<(i32, Vec<(String, String)>)>>,
    loading_defs: bool,
    promo: Vec<i32>,
    updates: HashMap<u64, Vec<(u64, String, Option<PropValue>)>>,
}

static I: LazyLock<Mutex<Inv>> = LazyLock::new(|| Mutex::new(Inv { next: 1, ..Default::default() }));

/// Lock order: this first, then the global state.
fn inv() -> MutexGuard<'static, Inv> {
    I.lock().unwrap_or_else(|e| e.into_inner())
}

fn me() -> u64 {
    lock().me()
}

/// Start `op`; the handle goes to `out` now, SteamInventoryResultReady_t follows.
fn start(out: *mut i32, op: InventoryOp) -> bool {
    let full = matches!(op, InventoryOp::All);
    let h = {
        let mut i = inv();
        let h = i.next;
        i.next += 1;
        i.results.insert(h, Res { status: PENDING, items: Vec::new(), ticket: Vec::new(), owner: 0, time: 0 });
        h
    };
    put(out, h);
    std::thread::spawn(move || {
        let (status, items, ticket) = match crate::ipc::request(Request::Inventory(op)) {
            Some(Reply::Inventory { eresult, items, ticket }) => (eresult, items, ticket),
            _ => (FAIL, Vec::new(), Vec::new()),
        };
        crate::log!("inventory: result {h}: {status}, {} item(s)", items.len());
        let owner = me();
        if let Some(r) = inv().results.get_mut(&h) {
            *r = Res { status, items, ticket, owner, time: now() };
        }
        let mut b = Bytes::new(8);
        b.i32(0, h).i32(4, status);
        let mut g = lock();
        g.post(RESULT_READY, b.0);
        if full && status == OK {
            let mut b = Bytes::new(4);
            b.i32(0, h);
            g.post(FULL_UPDATE, b.0);
        }
    });
    true
}

/// A result made right away (nothing to ask Steam).
fn ready(out: *mut i32, status: i32) -> bool {
    let h = {
        let mut i = inv();
        let h = i.next;
        i.next += 1;
        i.results.insert(h, Res { status, items: Vec::new(), ticket: Vec::new(), owner: 0, time: now() });
        h
    };
    put(out, h);
    let mut b = Bytes::new(8);
    b.i32(0, h).i32(4, status);
    lock().post(RESULT_READY, b.0);
    true
}

extern "C" fn status(_t: This, h: i32) -> i32 {
    inv().results.get(&h).map(|r| r.status).unwrap_or(FAIL)
}

/// SteamItemDetails_t: id, definition, quantity, flags (16 bytes).
extern "C" fn result_items(_t: This, h: i32, out: *mut u8, size: *mut u32) -> bool {
    let i = inv();
    let Some(r) = i.results.get(&h).filter(|r| r.status == OK) else { return false };
    if size.is_null() {
        return false;
    }
    let cap = unsafe { size.read_unaligned() } as usize;
    if out.is_null() {
        put(size, r.items.len() as u32);
        return true;
    }
    if cap < r.items.len() {
        put(size, r.items.len() as u32);
        return false;
    }
    for (k, it) in r.items.iter().enumerate() {
        let mut b = Bytes::new(16);
        b.u64(0, it.id).i32(8, it.def).u32(12, it.quantity as u32 | (it.flags as u32) << 16);
        unsafe { std::ptr::copy_nonoverlapping(b.0.as_ptr(), out.add(k * 16), 16) };
    }
    put(size, r.items.len() as u32);
    true
}

/// Steam's string-out convention: null buffer asks for the size (with the
/// NUL); otherwise copy what fits and report the bytes used.
fn string_out(s: &str, buf: *mut c_char, size: *mut u32) -> bool {
    if size.is_null() {
        return false;
    }
    let need = s.len() as u32 + 1;
    if buf.is_null() {
        put(size, need);
        return true;
    }
    let cap = unsafe { size.read_unaligned() };
    let n = crate::ifaces::copy_out(s, buf, cap as usize) as u32;
    put(size, n.max(1));
    cap >= need
}

fn property(props: &[(String, String)], name: *const c_char) -> String {
    if name.is_null() {
        return props.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(",");
    }
    let name = unsafe { text(name) };
    props.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone()).unwrap_or_default()
}

extern "C" fn result_property(_t: This, h: i32, index: u32, name: *const c_char, buf: *mut c_char, size: *mut u32) -> bool {
    let value = {
        let i = inv();
        let Some(it) = i.results.get(&h).and_then(|r| r.items.get(index as usize)) else { return false };
        property(&it.props, name)
    };
    string_out(&value, buf, size)
}

extern "C" fn timestamp(_t: This, h: i32) -> u32 {
    inv().results.get(&h).map(|r| r.time).unwrap_or(0)
}

extern "C" fn check_owner(_t: This, h: i32, steam_id: u64) -> bool {
    inv().results.get(&h).is_some_and(|r| r.owner == steam_id)
}

extern "C" fn destroy(_t: This, h: i32) {
    inv().results.remove(&h);
}

extern "C" fn all_items(_t: This, out: *mut i32) -> bool {
    load_defs_once();
    start(out, InventoryOp::All)
}

extern "C" fn items_by_id(_t: This, out: *mut i32, ids: *const u64, n: u32) -> bool {
    let ids = (0..n as usize).filter(|_| !ids.is_null()).map(|k| unsafe { ids.add(k).read_unaligned() }).collect();
    start(out, InventoryOp::ByIds(ids))
}

/// Steam's signed blob for this result (what the service returned).
extern "C" fn serialize(_t: This, h: i32, buf: *mut u8, size: *mut u32) -> bool {
    let i = inv();
    let Some(r) = i.results.get(&h).filter(|r| r.status == OK) else { return false };
    if size.is_null() {
        return false;
    }
    if buf.is_null() {
        put(size, r.ticket.len() as u32);
        return true;
    }
    let cap = unsafe { size.read_unaligned() } as usize;
    if cap < r.ticket.len() {
        put(size, r.ticket.len() as u32);
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(r.ticket.as_ptr(), buf, r.ticket.len()) };
    put(size, r.ticket.len() as u32);
    true
}

extern "C" fn deserialize(_t: This, out: *mut i32, _buf: *const u8, _size: u32, _reserved: bool) -> bool {
    crate::log!("inventory: can't verify another player's items");
    ready(out, FAIL)
}

fn pairs<A: Copy, B: Copy>(a: *const A, b: *const B, n: u32, default: B) -> Vec<(A, B)> {
    if a.is_null() {
        return Vec::new();
    }
    (0..n as usize).map(|k| unsafe { (a.add(k).read_unaligned(), if b.is_null() { default } else { b.add(k).read_unaligned() }) }).collect()
}

extern "C" fn generate(_t: This, out: *mut i32, defs: *const i32, qty: *const u32, n: u32) -> bool {
    start(out, InventoryOp::Generate(pairs(defs, qty, n, 1)))
}

extern "C" fn grant_promo(_t: This, out: *mut i32) -> bool {
    start(out, InventoryOp::Promo(Vec::new()))
}

extern "C" fn add_promo(_t: This, out: *mut i32, def: i32) -> bool {
    start(out, InventoryOp::Promo(vec![def]))
}

extern "C" fn add_promos(_t: This, out: *mut i32, defs: *const i32, n: u32) -> bool {
    let defs = pairs(defs, std::ptr::null::<u8>(), n, 0).into_iter().map(|(d, _)| d).collect();
    start(out, InventoryOp::Promo(defs))
}

extern "C" fn consume(_t: This, out: *mut i32, item: u64, quantity: u32) -> bool {
    start(out, InventoryOp::Consume { item, quantity })
}

#[allow(clippy::too_many_arguments)]
extern "C" fn exchange(_t: This, out: *mut i32, gen_defs: *const i32, _gen_qty: *const u32, gen_n: u32, destroy: *const u64, destroy_qty: *const u32, destroy_n: u32) -> bool {
    // Steam makes exactly one item per exchange.
    if gen_defs.is_null() || gen_n != 1 {
        return false;
    }
    let generate = unsafe { gen_defs.read_unaligned() };
    start(out, InventoryOp::Exchange { generate, destroy: pairs(destroy, destroy_qty, destroy_n, 1) })
}

extern "C" fn transfer(_t: This, out: *mut i32, source: u64, quantity: u32, dest: u64) -> bool {
    start(out, InventoryOp::Transfer { source, quantity, dest })
}

extern "C" fn drop(_t: This, out: *mut i32, def: i32) -> bool {
    start(out, InventoryOp::Drop { def })
}

// ------------------------------------------------------------ definitions

fn load_defs_once() {
    {
        let mut i = inv();
        if i.defs.is_some() || i.loading_defs {
            return;
        }
        i.loading_defs = true;
    }
    std::thread::spawn(|| {
        let defs = match crate::ipc::request(Request::ItemDefs) {
            Some(Reply::ItemDefs { defs }) => defs,
            _ => Vec::new(),
        };
        crate::log!("inventory: {} item definition(s)", defs.len());
        {
            let mut i = inv();
            i.defs = Some(defs);
            i.loading_defs = false;
        }
        lock().post(DEFINITION_UPDATE, vec![0]);
    });
}

extern "C" fn load_defs(_t: This) -> bool {
    load_defs_once();
    true
}

extern "C" fn def_ids(_t: This, out: *mut i32, size: *mut u32) -> bool {
    load_defs_once();
    let i = inv();
    let Some(defs) = i.defs.as_ref() else { return false };
    if size.is_null() {
        return false;
    }
    if out.is_null() {
        put(size, defs.len() as u32);
        return true;
    }
    let cap = unsafe { size.read_unaligned() } as usize;
    for (k, (id, _)) in defs.iter().take(cap).enumerate() {
        unsafe { out.add(k).write_unaligned(*id) };
    }
    put(size, defs.len().min(cap) as u32);
    cap >= defs.len()
}

extern "C" fn def_property(_t: This, def: i32, name: *const c_char, buf: *mut c_char, size: *mut u32) -> bool {
    load_defs_once();
    let value = {
        let i = inv();
        let Some(props) = i.defs.as_ref().and_then(|d| d.iter().find(|(id, _)| *id == def)).map(|(_, p)| p.clone()) else { return false };
        property(&props, name)
    };
    string_out(&value, buf, size)
}

/// SteamInventoryEligiblePromoItemDefIDs_t: EResult, user, count, cached.
extern "C" fn request_promo(_t: This, steam_id: u64) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let (eresult, defs) = match crate::ipc::request(Request::PromoItems) {
            Some(Reply::PromoItems { eresult, defs }) => (eresult, defs),
            _ => (FAIL, Vec::new()),
        };
        let n = defs.len() as i32;
        inv().promo = defs;
        let mut b = Bytes::new(24);
        b.i32(0, eresult).u64(8, steam_id).i32(16, n);
        lock().complete(call, ELIGIBLE_PROMO, b.0, false);
    });
    call
}

extern "C" fn promo_ids(_t: This, _steam_id: u64, out: *mut i32, size: *mut u32) -> bool {
    let i = inv();
    if size.is_null() {
        return false;
    }
    let cap = unsafe { size.read_unaligned() } as usize;
    for (k, id) in i.promo.iter().take(cap).enumerate() {
        if !out.is_null() {
            unsafe { out.add(k).write_unaligned(*id) };
        }
    }
    put(size, i.promo.len().min(cap) as u32);
    true
}

// ------------------------------------------------------ purchases (no)

fn failed_call(callback: i32, size: usize) -> u64 {
    let mut b = Bytes::new(size);
    b.i32(0, FAIL);
    let mut g = lock();
    let call = g.new_call();
    g.complete(call, callback, b.0, false);
    call
}

extern "C" fn start_purchase(_t: This) -> u64 {
    crate::log!("inventory: buying items needs Steam's overlay checkout (not supported)");
    failed_call(START_PURCHASE, 24)
}

extern "C" fn request_prices(_t: This) -> u64 {
    failed_call(REQUEST_PRICES, 8)
}

// ------------------------------------------------------ property updates

extern "C" fn start_update(_t: This) -> u64 {
    let mut i = inv();
    let h = i.next as u64 | 1 << 32;
    i.next += 1;
    i.updates.insert(h, Vec::new());
    h
}

fn change(h: u64, item: u64, name: *const c_char, value: Option<PropValue>) -> bool {
    let name = unsafe { text(name) };
    match inv().updates.get_mut(&h) {
        Some(list) if !name.is_empty() => {
            list.push((item, name, value));
            true
        }
        _ => false,
    }
}

extern "C" fn remove_property(_t: This, h: u64, item: u64, name: *const c_char) -> bool {
    change(h, item, name, None)
}

extern "C" fn set_string(_t: This, h: u64, item: u64, name: *const c_char, value: *const c_char) -> bool {
    let v = unsafe { text(value) };
    change(h, item, name, Some(PropValue::Str(v)))
}

extern "C" fn set_bool(_t: This, h: u64, item: u64, name: *const c_char, value: bool) -> bool {
    change(h, item, name, Some(PropValue::Bool(value)))
}

extern "C" fn set_int(_t: This, h: u64, item: u64, name: *const c_char, value: i64) -> bool {
    change(h, item, name, Some(PropValue::Int(value)))
}

extern "C" fn set_float(_t: This, h: u64, item: u64, name: *const c_char, value: f32) -> bool {
    change(h, item, name, Some(PropValue::Float(value)))
}

extern "C" fn submit_update(_t: This, h: u64, out: *mut i32) -> bool {
    let Some(list) = inv().updates.remove(&h) else { return false };
    start(out, InventoryOp::Modify(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_and_properties() {
        let h = 4242;
        inv().results.insert(
            h,
            Res {
                status: OK,
                items: vec![InvItem { id: 9, def: 100, quantity: 2, flags: 1, props: vec![("origin".into(), "playtime".into()), ("itemdefid".into(), "100".into())] }],
                ticket: vec![1, 2, 3],
                owner: 5,
                time: 1,
            },
        );
        let mut n = 0u32;
        assert!(result_items(std::ptr::null_mut(), h, std::ptr::null_mut(), &mut n));
        assert_eq!(n, 1);
        let mut out = [0u8; 16];
        assert!(result_items(std::ptr::null_mut(), h, out.as_mut_ptr(), &mut n));
        assert_eq!(u64::from_le_bytes(out[0..8].try_into().unwrap()), 9);
        assert_eq!(i32::from_le_bytes(out[8..12].try_into().unwrap()), 100);
        assert_eq!((u16::from_le_bytes(out[12..14].try_into().unwrap()), u16::from_le_bytes(out[14..16].try_into().unwrap())), (2, 1));
        let mut size = 0u32;
        assert!(result_property(std::ptr::null_mut(), h, 0, std::ptr::null(), std::ptr::null_mut(), &mut size));
        let mut buf = vec![0 as c_char; size as usize];
        assert!(result_property(std::ptr::null_mut(), h, 0, std::ptr::null(), buf.as_mut_ptr(), &mut size));
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap(), "origin,itemdefid");
        assert!(check_owner(std::ptr::null_mut(), h, 5));
        let mut s = 0u32;
        assert!(serialize(std::ptr::null_mut(), h, std::ptr::null_mut(), &mut s));
        assert_eq!(s, 3);
        destroy(std::ptr::null_mut(), h);
        assert_eq!(status(std::ptr::null_mut(), h), FAIL);
    }
}

// Trade Helper: an example SteamInRust plugin.
// Shows the trade API: it notifies you about new offers with the market
// value of both sides, and lists your incoming offers with buttons to
// review (the app asks before anything is accepted) or decline them.

// Market value of a list of items, in cents, and how many have no price yet.
fn worth(items) {
    let total = 0;
    let unknown = 0;
    for i in items {
        if i.price == () {
            if i.marketable {
                unknown += 1;
                market_price(i.app_id, i.market_hash_name);
            }
        } else {
            total += i.price * i.amount;
        }
    }
    #{ total: total, unknown: unknown }
}

fn money(w) {
    let s = (w.total / 100) + "." + if w.total % 100 < 10 { "0" + (w.total % 100) } else { "" + (w.total % 100) };
    s + " " + currency() + if w.unknown > 0 { " (+" + w.unknown + " not priced yet)" } else { "" }
}

fn on_load() { refresh_trades(); }

fn on_event(name, data) {
    if name == "trade_offer" {
        for o in trade_offers() {
            if o.id == data.id {
                notify(data.name + " sent a trade offer: you get " + o.get.len() + " items (" + money(worth(o.get)) + "), you give " + o.give.len() + " (" + money(worth(o.give)) + ")");
            }
        }
    } else if name == "trade_changed" {
        log("offer " + data.id + " is now " + data.state);
    }
}

fn page() {
    let out = [#{type: "heading", text: "Incoming trade offers"}];
    let any = false;
    for o in trade_offers() {
        if o.ours || o.state != "active" { continue; }
        any = true;
        let get = worth(o.get);
        let give = worth(o.give);
        out.push(#{type: "separator"});
        out.push(#{type: "text", text: o.partner_name + (if o.message == "" { "" } else { ": \"" + o.message + "\"" })});
        out.push(#{type: "row", items: [
            #{type: "stat", label: "You get", value: o.get.len() + " items · " + money(get)},
            #{type: "stat", label: "You give", value: o.give.len() + " items · " + money(give)},
        ]});
        if get.total > give.total && give.unknown == 0 {
            out.push(#{type: "pill", text: "Worth more than you give"});
        }
        out.push(#{type: "row", items: [
            #{type: "button", id: "review:" + o.id, text: "Review"},
            #{type: "button", id: "decline:" + o.id, text: "Decline"},
        ]});
    }
    if !any { out.push(#{type: "muted", text: "No offers waiting. New ones show up here (checked every few minutes)."}); }
    out.push(#{type: "separator"});
    out.push(#{type: "button", id: "refresh", text: "Check now"});
    out
}

fn on_click(id) {
    if id == "refresh" { refresh_trades(); return; }
    let parts = id.split(":");
    if parts[0] == "review" { review_trade(parts[1]); }
    if parts[0] == "decline" { decline_trade(parts[1]); }
}

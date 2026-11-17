// Wishlist Sales: an example SteamInRust plugin.
// Every hour it asks Steam's store for your wishlist's prices, keeps the
// games on sale, and notifies you about new discounts. Shows the network,
// timer, table and game-page button APIs.

fn check() {
    let ids = wishlist();
    if ids.len() == 0 { return; }
    // Steam's store answers up to about 100 games at a time with prices only.
    let list = "";
    for id in ids {
        if list.len() > 0 { list += ","; }
        list += id.to_string();
    }
    storage_set("checking", "yes");
    http_get("https://store.steampowered.com/api/appdetails?filters=price_overview&appids=" + list);
}

fn on_load() {
    // A first look right away (then every hour, from on_tick).
    if wishlist().len() > 0 { check(); }
}

fn on_tick() { check(); }

fn on_http(id, status, body) {
    storage_set("checking", "no");
    if status != 200 {
        log("store answered " + status);
        return;
    }
    let data = json_parse(body);
    let seen = storage_get("seen");
    let seen = if seen == () { #{} } else { json_parse(seen) };
    let sales = [];
    for app in data.keys() {
        let entry = data[app];
        if entry.success != true || type_of(entry.data) != "map" { continue; }
        let p = entry.data.price_overview;
        if p == () || p.discount_percent == 0 { continue; }
        sales.push(#{ app_id: parse_int(app), discount: p.discount_percent, price: p.final_formatted });
        // Tell once per discount.
        let key = app + ":" + p.discount_percent;
        if seen[key] == () {
            seen[key] = true;
            notify("A wishlist game is " + p.discount_percent + "% off: " + p.final_formatted);
        }
    }
    storage_set("seen", json_string(seen));
    storage_set("sales", json_string(sales));
    storage_set("checked", now().to_string());
}

fn sales() {
    let s = storage_get("sales");
    if s == () { [] } else { json_parse(s) }
}

fn page() {
    let sales = sales();
    let items = [
        #{ type: "heading", text: "Wishlist Sales" },
        #{ type: "muted", text: "Games on your wishlist that are on sale right now. Checked every hour." },
        #{ type: "row", items: [
            #{ type: "stat", label: "On sale", value: sales.len().to_string() },
            #{ type: "stat", label: "On your wishlist", value: wishlist().len().to_string() },
        ]},
        #{ type: "button", id: "check", text: if storage_get("checking") == "yes" { "Checking…" } else { "Check now" } },
    ];
    if sales.len() == 0 {
        items.push(#{ type: "muted", text: "Nothing on sale right now." });
        return items;
    }
    let rows = [];
    for s in sales {
        rows.push([s.discount + "%", s.price, s.app_id.to_string()]);
    }
    items.push(#{ type: "table", columns: ["Off", "Price", "App"], rows: rows });
    for s in sales {
        items.push(#{ type: "game", app_id: s.app_id });
    }
    items
}

fn on_click(id) {
    if id == "check" { check(); }
}

// A button on every game's page in the library.
fn game_actions(app_id) {
    [#{ id: "copy-link", text: "Copy store link" }]
}

fn on_game_action(id, app_id) {
    if id == "copy-link" {
        copy("https://store.steampowered.com/app/" + app_id + "/");
        notify("Store link copied");
    }
}

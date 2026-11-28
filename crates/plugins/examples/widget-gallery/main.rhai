// Widget Gallery: an example SteamInRust plugin.
// Shows every widget, in the normal window and in Big Picture. Copy the
// pieces you need. See docs/plugin-api.md for what each one takes.

fn saved(key, fallback) {
    let v = storage_get(key);
    if v == () { fallback } else { v }
}

fn page() {
    let tab = saved("tab", "basics");
    // A few of your games (installed ones first).
    let games = installed_games();
    if games.len() < 4 { games += library().filter(|g| !g.installed).map(|g| game_info(g.app_id)); }
    games.truncate(4);
    let items = [
        #{ type: "muted", text: "Shown in " + view_mode() + ". Pick a tab to see a group of widgets." },
        #{ type: "tabs", id: "tab", value: tab, items: [
            #{ value: "basics", label: "Basics" },
            #{ value: "inputs", label: "Inputs" },
            #{ value: "data", label: "Data" },
            #{ value: "layout", label: "Layout" },
        ]},
    ];
    if tab == "basics" {
        items += [
            #{ type: "heading", text: "Text and labels" },
            #{ type: "text", text: "Plain text wraps to the width of the page." },
            #{ type: "muted", text: "Muted text is for notes." },
            #{ type: "row", items: [
                #{ type: "badge", text: "Good", tone: "good" },
                #{ type: "badge", text: "Careful", tone: "warn" },
                #{ type: "badge", text: "Bad", tone: "bad" },
                #{ type: "pill", text: "A pill" },
            ]},
            #{ type: "row", items: [
                #{ type: "tooltip", text: "This button counts presses.", item: #{ type: "button", id: "count", text: "Pressed " + saved("count", "0") + " times" } },
                #{ type: "link", text: "Open the store", page: "store" },
            ]},
            #{ type: "separator" },
            #{ type: "loading", text: "Something loading looks like this" },
        ];
    } else if tab == "inputs" {
        items += [
            #{ type: "input", id: "name", value: saved("name", ""), hint: "Type a name" },
            #{ type: "toggle", id: "on", text: "A switch", value: saved("on", "false") == "true" },
            #{ type: "select", id: "sort", label: "Sort by", value: saved("sort", "name"), options: [
                #{ value: "name", label: "Name" },
                #{ value: "playtime", label: "Play time" },
                #{ value: "size", label: "Size" },
            ]},
            #{ type: "slider", id: "level", label: "Level", min: 0, max: 10, step: 1, value: parse_int(saved("level", "5")) },
            #{ type: "text", text: "Saved: " + saved("name", "(no name)") + ", sort by " + saved("sort", "name") + ", level " + saved("level", "5") },
        ];
    } else if tab == "data" {
        let minutes = games.map(|g| g.playtime_minutes / 60);
        items += [
            #{ type: "grid", columns: 3, items: [
                #{ type: "metric", label: "Installed", value: games.len().to_string(), hint: "games on this PC" },
                #{ type: "metric", label: "This week", value: "12 h", change: "+3 h" },
                #{ type: "stat", label: "Library", value: library().len().to_string() },
            ]},
            #{ type: "chart", kind: "bar", values: minutes, labels: games.map(|g| g.name) },
            #{ type: "row", items: [ #{ type: "text", text: "A sparkline:" }, #{ type: "sparkline", values: [3, 5, 4, 8, 6, 9, 7] } ] },
            #{ type: "progress", value: 0.6 },
            #{ type: "table", columns: ["Game", "Hours"], rows: games.map(|g| [g.name, (g.playtime_minutes / 60).to_string()]) },
        ];
    } else {
        items += [
            #{ type: "section", title: "A section", items: [ #{ type: "text", text: "A section is a titled group." } ] },
            #{ type: "card", title: "A card", items: [
                #{ type: "text", text: "A card is a box around other widgets." },
                #{ type: "button", id: "count", text: "A button in a card" },
            ]},
            #{ type: "grid", columns: 2, items: games.map(|g| #{ type: "game", app_id: g.app_id }) },
            #{ type: "empty_state", icon: "trophy", title: "Nothing here yet", text: "An empty state, for when a list is empty." },
        ];
    }
    items
}

fn on_select(id, value) { storage_set(id, value); }
fn on_input(id, text) { storage_set(id, text); }
fn on_toggle(id, on) { storage_set(id, on.to_string()); }
fn on_slider(id, value) { storage_set(id, value.to_int().to_string()); }
fn on_click(id) {
    if id == "count" { storage_set("count", (parse_int(saved("count", "0")) + 1).to_string()); }
}

// A button on every game's page (in both modes), and in friends' menus.
fn game_actions(app_id) { [#{ id: "mark", text: "Mark as favorite in the gallery" }] }
fn on_game_action(id, app_id) { storage_set("marked", app_id.to_string()); }

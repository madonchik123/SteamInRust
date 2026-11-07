// Backlog Picker: an example SteamInRust plugin.
// Lists installed games with no playtime and picks one at random.

fn backlog() {
    library().filter(|g| g.installed && g.playtime_minutes == 0)
}

fn page() {
    let games = backlog();
    let total = library().len();
    let items = [
        #{ type: "heading", text: "Backlog" },
        #{ type: "muted", text: "Installed games you haven't played yet." },
        #{ type: "row", items: [
            #{ type: "stat", label: "Unplayed", value: games.len().to_string() },
            #{ type: "stat", label: "In library", value: total.to_string() },
        ]},
        #{ type: "progress", value: if total == 0 { 0.0 } else { 1.0 - games.len().to_float() / total.to_float() } },
        #{ type: "button", id: "pick", text: "Pick one for me" },
    ];

    let pick = storage_get("pick");
    if pick != () {
        items.push(#{ type: "separator" });
        items.push(#{ type: "heading", text: "Tonight's pick" });
        items.push(#{ type: "game", app_id: parse_int(pick) });
    }
    items
}

fn on_click(id) {
    if id == "pick" {
        let games = backlog();
        if games.len() > 0 {
            let i = now() % games.len();
            storage_set("pick", games[i].app_id.to_string());
        }
    }
}

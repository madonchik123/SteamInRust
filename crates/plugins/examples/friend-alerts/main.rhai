// Friend Alerts: an example SteamInRust plugin.
// Watches friends coming online / starting games and shows notifications.

let recent = [];

fn watching_everyone() {
    let v = storage_get("everyone");
    v == () || v == "true"
}

fn on_event(name, data) {
    let text = if name == "friend_online" {
        data.name + " is now online"
    } else if name == "friend_game" && data.game != () {
        data.name + " started playing " + data.game
    } else {
        return;
    };

    if watching_everyone() {
        notify(text);
    }
    recent.insert(0, text);
    if recent.len() > 20 { recent.truncate(20); }
}

fn page() {
    let online = friends().filter(|f| f.status != "Offline");
    let items = [
        #{ type: "heading", text: "Friend Alerts" },
        #{ type: "muted", text: "Get a notification when friends come online or start playing." },
        #{ type: "row", items: [
            #{ type: "stat", label: "Friends online", value: online.len().to_string() },
            #{ type: "stat", label: "Alerts", value: if watching_everyone() { "On" } else { "Off" } },
        ]},
        #{ type: "button", id: "toggle", text: if watching_everyone() { "Turn alerts off" } else { "Turn alerts on" } },
        #{ type: "separator" },
        #{ type: "heading", text: "Recent" },
    ];
    if recent.len() == 0 {
        items.push(#{ type: "muted", text: "Nothing yet." });
    }
    for r in recent {
        items.push(#{ type: "text", text: r });
    }
    items
}

fn on_click(id) {
    if id == "toggle" {
        storage_set("everyone", if watching_everyone() { "false" } else { "true" });
    }
}

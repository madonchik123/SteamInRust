//! Steam Support (help.steampowered.com). It has no API, so your requests
//! and their messages are read from its pages with your signed-in session.

/// Text between `start` and the next `end` after `from`, with tags removed.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ").replace('\u{FFFD}', "")
}

pub fn parse_tickets(body: &str) -> Vec<model::SupportTicket> {
    body.split("class=\"help_request_row")
        .skip(1)
        .filter_map(|row| {
            let url = between(row, "href=\"", "\"")?.to_string();
            let id = url.rsplit('/').next().unwrap_or_default().to_string();
            Some(model::SupportTicket {
                id,
                open: !row.starts_with(" closed"),
                updated: clean(between(row, "class=\"col date_created\">", "</div>").unwrap_or_default()),
                status: clean(between(row, "class=\"col status\">", "</div>").unwrap_or_default()),
                title: clean(between(row, "class=\"help_request_name ellipsis\">", "</div>").unwrap_or_default()),
                area: clean(between(row, "class=\"help_request_area ellipsis\">", "</div>").unwrap_or_default()),
                icon: between(row, "class=\"help_request_icon\" src=\"", "\"").map(String::from),
                url,
            })
        })
        .collect()
}

/// The conversation on a support request's page, oldest first.
pub fn parse_messages(body: &str) -> Vec<model::SupportMessage> {
    body.split("help_request_message")
        .skip(1)
        .filter_map(|block| {
            let from_steam = block.starts_with(" from_steam");
            let header = clean(between(block, "class=\"author_bar_text\">", "<div style")?).trim().to_string();
            // Message bodies are BBCode-style HTML without nested <div>s.
            let html = between(block, "class=\"help_request_text\">", "</div>")?.trim().to_string();
            Some(model::SupportMessage { from_steam, header, html })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_messages() {
        let page = r#"<div class="help_request_initial_body help_request_message"> <div class="help_request_author_bar"> <div class="author_bar_text"> <div class="help_request_author_avatar"></div> Message from you on Oct 7 @ 12:09pm | 4 weeks ago </div> <div style="clear: left;"></div> </div> <div class="help_request_text"> Please refund </div> </div> <div class="help_request_message from_steam"> <div class="help_request_author_bar"> <div class="author_bar_text"> Message from Steam Support on Oct 9 @ 2:43pm </div> <div style="clear: left;"></div> </div> <div class="help_request_text"> <b>Sorry</b> </div> </div> <div class="help_request_footerbox"><div>Reply</div></div>"#;
        let m = super::parse_messages(page);
        assert_eq!(m.len(), 2);
        assert!(!m[0].from_steam && m[1].from_steam);
        assert!(m[0].header.starts_with("Message from you"), "{}", m[0].header);
        assert_eq!(m[0].html, "Please refund");
        assert_eq!(m[1].html, "<b>Sorry</b>");
    }

    #[test]
    fn reads_requests() {
        let page = r#"<a class="help_request_row closed" href="https://help.steampowered.com/en/wizard/HelpRequest/HT-AAAA-BBBB">
            <div class="help_request_columns"><div class="col date_created">Oct 7</div><div class="col status">Closed</div>
            <div class="col message"><img class="help_request_icon" src="https://x/570.jpg">
            <div class="help_request_name ellipsis"> Refund Request - Accidentally purchased twice </div>
            <div class="help_request_area ellipsis"> Dota 2 </div></div></div></a>"#;
        let t = super::parse_tickets(page);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "HT-AAAA-BBBB");
        assert_eq!(t[0].title, "Refund Request - Accidentally purchased twice");
        assert_eq!(t[0].area, "Dota 2");
        assert_eq!(t[0].status, "Closed");
        assert!(!t[0].open);
    }
}

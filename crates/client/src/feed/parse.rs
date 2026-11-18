//! Reading the feeds' HTML. Steam has no API for either, so this follows
//! the pages' classes and keeps going when a part is missing.

use super::{Feed, Post, PostKind};
use crate::hub::{decode, strip_tags};
use model::SteamId;

const BASE_ID: u64 = 76561197960265728;

/// The answer of `ajaxgetusernews`: JSON with the feed's HTML (one day) and
/// the link to the day before.
pub fn parse_activity(body: &str) -> Result<Feed, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "Steam didn't send the activity feed".to_string())?;
    if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
        return Err("Steam didn't send the activity feed".into());
    }
    let next = v
        .get("next_request")
        .and_then(|n| n.as_str())
        .and_then(|n| n.split("start=").nth(1))
        .and_then(|n| n.split('&').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let html = v.get("blotter_html").and_then(|h| h.as_str()).unwrap_or_default();
    Ok(Feed { posts: parse_blotter(html), next })
}

/// The activity feed's HTML: days, each with a roll-up of small events
/// ("X and Y are now friends") and blocks for bigger ones (a review, a
/// shared file, a purchase, a status, an announcement).
pub fn parse_blotter(html: &str) -> Vec<Post> {
    let mut posts = Vec::new();
    for day in html.split("class=\"blotter_day\"").skip(1) {
        let when = div_inner(day, "blotter_day_header_date").map(text).unwrap_or_default();
        for block in day.split("class=\"blotter_block\"").skip(1) {
            if let Some(rollup) = div_inner(block, "blotter_daily_rollup") {
                for line in rollup.split("class=\"blotter_daily_rollup_line\"").skip(1) {
                    posts.extend(rollup_line(line, &when));
                }
            } else {
                posts.extend(blotter_block(block, &when));
            }
        }
    }
    posts
}

fn rollup_line(line: &str, when: &str) -> Option<Post> {
    let span = between(line, "<span>", "</span>")?;
    let action = text(span);
    if action.is_empty() {
        return None;
    }
    let author = miniprofile(line);
    let author_name = between(span, "data-miniprofile=\"", "</a>").and_then(|s| s.split_once('>')).map(|(_, n)| text(n)).unwrap_or_default();
    let avatar = img_srcs(div_inner(line, "blotter_rollup_avatar").unwrap_or_default()).into_iter().find(|s| s.contains("avatars."));
    // Achievement icons come after the line, named in their `title`.
    let tail = line.split_once("</span>").map(|x| x.1).unwrap_or_default();
    let images: Vec<String> = img_srcs(tail).into_iter().filter(|s| !s.contains("avatars.")).collect();
    let names: Vec<String> = tail.split("<img").skip(1).filter_map(|t| attr(t.split('>').next().unwrap_or_default(), "title")).map(|t| t.trim().to_string()).collect();
    let lower = action.to_lowercase();
    let kind = if span.contains("achievement") || lower.contains("achieved") || lower.contains("unlocked") {
        PostKind::Achievement
    } else if lower.contains("now owns") || lower.contains("purchased") {
        PostKind::Purchase
    } else {
        PostKind::Status
    };
    let url = links(span).into_iter().find(|l| l.contains("/app/") || l.contains("filedetails")).unwrap_or_default();
    let text = names.join("

");
    Some(Post { kind, author, author_name, avatar, app_id: app_link(span), action, text, images, url, when: when.to_string(), ..Default::default() })
}

fn blotter_block(block: &str, when: &str) -> Option<Post> {
    let block = block.split_once('>')?.1;
    let class = between(block, "class=\"", "\"")?.split_whitespace().next()?.to_string();
    let mut p = Post { when: when.to_string(), ..Default::default() };
    let head = block.split('>').next().unwrap_or_default();
    p.app_id = attr(head, "data-appid").and_then(|a| a.parse().ok()).unwrap_or(0);
    if let Some(author) = div_inner(block, "blotter_author_block") {
        p.author = miniprofile(author);
        if p.author == 0 {
            p.author = profile_id(author);
        }
        p.avatar = img_srcs(author).into_iter().find(|s| s.contains("avatars."));
        // The author's name, then what they did ("reviewed <game>").
        let rest = match author.rfind("blotter_avatar_holder") {
            Some(_) => after_div(author, "blotter_avatar_holder"),
            None => author,
        };
        let rest = if rest.contains("blotter_userstatus_game") { after_div(rest, "blotter_userstatus_game") } else { rest };
        let names = links_text(rest);
        p.author_name = names.first().cloned().unwrap_or_default();
        let all = text(rest);
        p.action = all.strip_prefix(p.author_name.as_str()).unwrap_or(&all).trim().to_string();
        if p.app_id == 0 {
            p.app_id = app_link(author);
        }
    }
    p.kind = match class.as_str() {
        "blotter_gamepurchase" => PostKind::Purchase,
        "blotter_recommendation" => PostKind::Review,
        "blotter_userstatus" if block.contains("blotter_group_announcement") => PostKind::Announcement,
        "blotter_userstatus" => PostKind::Status,
        "blotter_workshopitempublished" => {
            let kind = classes_of(block, "blotter_workshopitem");
            if block.contains("blotter_webguide") || kind.iter().any(|k| k.contains("guide")) {
                PostKind::Guide
            } else if kind.iter().any(|k| k == "artwork") {
                PostKind::Artwork
            } else if kind.iter().any(|k| k == "screenshot") {
                PostKind::Screenshot
            } else if kind.iter().any(|k| k == "video") {
                PostKind::Video
            } else {
                PostKind::Workshop
            }
        }
        c if c.contains("achievement") => PostKind::Achievement,
        c if c.contains("screenshot") => PostKind::Screenshot,
        _ => PostKind::Other,
    };
    match p.kind {
        PostKind::Purchase => {
            if let Some(c) = div_inner(block, "blotter_gamepurchase_content") {
                p.images = img_srcs(c);
                p.text = between(c, "class=\"blotter_gamepurchase_text\">", "<").map(text).unwrap_or_default();
                p.url = links(c).into_iter().next().unwrap_or_default();
                if p.app_id == 0 {
                    p.app_id = between(c, "data-appid=\"", "\"").and_then(|a| a.parse().ok()).unwrap_or(0);
                }
            }
        }
        PostKind::Review => {
            if let Some(c) = div_inner(block, "blotter_recommendation_content") {
                review_fields(&mut p, c);
            }
        }
        PostKind::Announcement => {
            let head = div_inner(block, "blotter_group_announcement_header_text").unwrap_or_default();
            p.game = links_text(head).into_iter().next().unwrap_or_default();
            p.author_name = p.game.clone();
            p.avatar = img_srcs(div_inner(block, "blotter_group_announcement_header_ogg").unwrap_or_default()).into_iter().next();
            if p.app_id == 0 {
                p.app_id = app_link(head);
            }
            if let Some(h) = div_inner(block, "blotter_group_announcement_headline") {
                p.title = text(h);
                p.url = links(h).into_iter().next().unwrap_or_default();
            }
            if let Some(body) = div_inner(block, "group_announcement_auto_collapse") {
                p.images = img_srcs(body).into_iter().take(1).collect();
                p.text = shorten(&text_lines(body), 600);
            }
        }
        PostKind::Status => {
            p.text = div_inner(block, "blotter_userstatus_content").map(text_lines).unwrap_or_default();
            p.url = links(block).into_iter().find(|l| l.contains("/status/")).unwrap_or_default();
        }
        _ => {
            let item = div_inner(block, "blotter_workshopitem").or_else(|| div_inner(block, "blotter_screenshot")).unwrap_or(block);
            p.images = img_srcs(item).into_iter().filter(|s| !s.contains("avatars.") && !s.contains("community_assets")).collect();
            p.title = div_inner(item, "blotter_workshopitem_title").map(text).unwrap_or_default();
            p.text = div_inner(item, "blotter_workshopitem_description").map(text_lines).unwrap_or_default();
            p.file_id = between(item, "data-publishedfileid=\"", "\"").and_then(|a| a.parse().ok()).unwrap_or(0);
            p.url = links(item).into_iter().find(|l| l.contains("filedetails")).unwrap_or_default();
        }
    }
    if let Some(v) = between(block, "class=\"blotter_voters_names\"", "</span>") {
        let v = text(v.split_once('>').map(|x| x.1).unwrap_or_default());
        if v.contains("rated this up") {
            p.votes_up = first_number(&v).max(1);
        }
    }
    if let Some(c) = between(block, "View all ", " comment") {
        p.comments = first_number(c);
    }
    (!p.action.is_empty() || !p.title.is_empty() || !p.text.is_empty() || !p.images.is_empty()).then_some(p)
}

/// A review card's parts (a review in the feed, or on the community home).
fn review_fields(p: &mut Post, c: &str) {
    if let Some(t) = div_inner(c, "title") {
        p.recommended = Some(!text(t).to_lowercase().starts_with("not "));
    }
    if c.contains("icon_thumbsDown") {
        p.recommended = Some(false);
    }
    p.hours = div_inner(c, "hours").map(text).unwrap_or_default();
    if let Some(body) = div_inner(c, "apphub_CardTextContent") {
        let body = match div_inner(body, "date_posted") {
            Some(d) => {
                p.when = text(d);
                after_div(body, "date_posted")
            }
            None => body,
        };
        p.text = text_lines(body);
    }
    if let Some(h) = div_inner(c, "found_helpful") {
        // "579 people found this review helpful<br>6 people found this review funny"
        let h = h.split("<div").next().unwrap_or_default();
        for line in h.split("<br>") {
            let line = text(line);
            if line.contains("funny") {
                p.funny = first_number(&line);
            } else if line.contains("helpful") {
                p.votes_up = first_number(&line);
            }
        }
    }
    if p.url.is_empty() {
        p.url = links(c).into_iter().find(|l| l.contains("/recommended/")).unwrap_or_default();
    }
}

/// Steam's community-wide feed (`allcontenthome`): cards of screenshots,
/// artwork, videos, broadcasts, Workshop items, guides and reviews.
pub fn parse_explore(html: &str) -> Vec<Post> {
    let mut starts: Vec<usize> = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find("apphub_Card").map(|i| i + from) {
        from = i + "apphub_Card".len();
        let next = html[from..].chars().next();
        let in_class = html[..i].rfind("class=\"").is_some_and(|c| !html[c + 7..i].contains('"'));
        if in_class && matches!(next, Some(' ' | '"')) {
            starts.push(html[..i].rfind("<div").unwrap_or(i));
        }
    }
    let mut posts = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(html.len());
        posts.extend(explore_card(&html[s..end]));
    }
    posts
}

fn explore_card(card: &str) -> Option<Post> {
    let mut p = Post::default();
    let head = card.split('>').next().unwrap_or_default();
    p.url = attr(head, "data-modal-content-url").unwrap_or_default();
    p.file_id = attr(card, "data-publishedfileid").and_then(|a| a.parse().ok()).unwrap_or(0);
    p.app_id = attr(card, "data-appid").and_then(|a| a.parse().ok()).unwrap_or(0);
    if let Some(name) = div_inner(card, "apphub_CardContentAppName") {
        p.game = text(name);
        if p.app_id == 0 {
            p.app_id = app_link(name);
        }
    }
    let label = div_inner(card, "apphub_CardContentType").map(text).unwrap_or_default();
    if let Some((game, _)) = label.rsplit_once(": ") {
        if p.game.is_empty() {
            p.game = game.trim().to_string();
        }
    }
    // The kind from the card's markup: Steam writes the label in the
    // reader's language. Screenshots and artwork look the same; the label
    // tells them apart in English, Steam's file details in any language.
    let kind = label.rsplit(": ").next().unwrap_or_default().to_lowercase();
    let classes = attr(head, "class").unwrap_or_default();
    let has_class = |c: &str| classes.split_whitespace().any(|x| x == c);
    p.kind = if has_class("Announcement_Card") || card.contains("apphub_CardContentNewsTitle") {
        PostKind::Announcement
    } else if card.contains("apphub_CardContentDiscussionTitle") {
        PostKind::Discussion
    } else if card.contains("apphub_UserReviewCardContent") {
        PostKind::Review
    } else if has_class("Broadcast_Card") {
        PostKind::Broadcast
    } else if card.contains("apphub_CardContentGuideTitle") || classes_of(card, "apphub_CardContentType").iter().any(|c| c == "guide") {
        PostKind::Guide
    } else if card.contains("apphub_CardContentPlayVideoIcon") {
        PostKind::Video
    } else if classes_of(card, "apphub_CardBG").iter().any(|c| c == "workshop") {
        PostKind::Workshop
    } else if kind.contains("artwork") {
        PostKind::Artwork
    } else if card.contains("apphub_CardContentPreviewImage") {
        PostKind::Screenshot
    } else if has_class("ugc") {
        PostKind::Workshop
    } else {
        PostKind::Other
    };
    let preview = tag_of(card, "apphub_CardContentPreviewImage").or_else(|| tag_of(card, "apphub_CardContentGuideImage")).and_then(|t| attr(t, "src"));
    p.images = preview.into_iter().filter(|s| s.starts_with("https://")).collect();
    // The author (Steam shows one on some cards only).
    if let Some(block) = div_inner(card, "apphub_CardContentAuthorBlock") {
        p.author = miniprofile(block);
        if p.author == 0 {
            p.author = profile_id(block);
        }
        p.author_name = div_inner(block, "apphub_CardContentAuthorName").map(text).unwrap_or_default();
        p.avatar = img_srcs(block).into_iter().find(|s| s.contains("avatars."));
    }
    match p.kind {
        PostKind::Announcement => {
            p.title = div_inner(card, "apphub_CardContentNewsTitle").map(text).unwrap_or_default();
            p.when = div_inner(card, "apphub_CardContentNewsDate").map(text).unwrap_or_default();
            if p.app_id == 0 {
                // Steam quotes this one with single quotes.
                let id = attr(head, "data-partner-event-appid").or_else(|| between(head, "data-partner-event-appid='", "'").map(String::from));
                p.app_id = id.and_then(|a| a.parse().ok()).unwrap_or(0);
            }
            if let Some(body) = div_inner(card, "apphub_CardTextContent") {
                p.images = img_srcs(body).into_iter().take(1).collect();
                p.text = shorten(&text_lines(body), 600);
            }
        }
        PostKind::Discussion => {
            p.title = div_inner(card, "apphub_CardContentDiscussionTitle").map(text).unwrap_or_default();
            p.text = div_inner(card, "apphub_CardTextContent").map(text_lines).map(|t| shorten(&t, 600)).unwrap_or_default();
            // Which forum it's in, where a friend's post says what they did.
            p.action = div_inner(card, "apphub_CardContentDiscussionType").map(text).unwrap_or_default();
            if let Some(block) = div_inner(card, "apphub_CardContentDiscussionAuthorBlock") {
                p.author = miniprofile(block);
                if p.author == 0 {
                    p.author = profile_id(block);
                }
                p.author_name = div_inner(block, "apphub_CardContentAuthorName").map(text).unwrap_or_default();
                p.avatar = img_srcs(block).into_iter().find(|s| s.contains("avatars."));
                p.when = div_inner(block, "apphub_CardContentDiscussionDate").map(text).unwrap_or_default();
            }
        }
        PostKind::Review => {
            review_fields(&mut p, card);
            if p.author == 0 {
                p.author = profile_id(&format!("href=\"{}\"", p.url));
            }
        }
        PostKind::Guide => {
            p.title = div_inner(card, "apphub_CardContentGuideTitle").map(text).unwrap_or_default();
            p.text = div_inner(card, "apphub_CardContentGuideDesc").map(text).unwrap_or_default();
        }
        PostKind::Broadcast => {
            p.live = true;
            p.views = div_inner(card, "apphub_CardContentViewers").map(|v| first_number(&text(v))).unwrap_or(0);
            p.title = div_inner(card, "apphub_CardContentTitle").map(text).unwrap_or_default();
            if p.url.is_empty() {
                p.url = links(card).into_iter().find(|l| l.contains("/broadcast/")).unwrap_or_default();
            }
        }
        _ => p.title = div_inner(card, "apphub_CardContentTitle").map(text).unwrap_or_default(),
    }
    if p.kind != PostKind::Review {
        if let Some(r) = div_inner(card, "apphub_CardRating").map(text) {
            if !r.contains("rating") {
                p.votes_up = first_number(&r);
            }
        }
    }
    p.comments = div_inner(card, "apphub_CardCommentCount").map(|c| first_number(&text(c))).unwrap_or(0);
    // Never a card with nothing in it.
    let something = !p.title.is_empty() || !p.text.is_empty() || !p.images.is_empty();
    (something && (!p.url.is_empty() || !p.images.is_empty())).then_some(p)
}

// ---------- small HTML helpers ----------

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

/// An attribute's value (whole names only: `src` doesn't match `data-src`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let mut from = 0;
    while let Some(i) = tag[from..].find(&needle).map(|i| i + from) {
        if i == 0 || tag[..i].ends_with(|c: char| c.is_whitespace()) {
            return between(&tag[i..], &needle, "\"").map(decode);
        }
        from = i + needle.len();
    }
    None
}

/// Position of the first element with this class: (start of the `<tag`,
/// the tag's name, end of its opening tag).
fn find_class(html: &str, class: &str) -> Option<(usize, String, usize)> {
    let mut from = 0;
    loop {
        let i = html[from..].find("class=\"").map(|i| i + from)? + 7;
        let end = html[i..].find('"')? + i;
        from = end;
        if !html[i..end].split_whitespace().any(|c| c == class) {
            continue;
        }
        let open = html[..i].rfind('<')?;
        let tag = html[open + 1..].split(|c: char| c.is_whitespace() || c == '>').next()?.to_string();
        let body = html[end..].find('>')? + end + 1;
        return Some((open, tag, body));
    }
}

/// The opening tag of the first element with this class.
fn tag_of<'a>(html: &'a str, class: &str) -> Option<&'a str> {
    let (open, _, body) = find_class(html, class)?;
    Some(&html[open..body])
}

/// The inside of the first element with this class (nested ones matched).
fn div_inner<'a>(html: &'a str, class: &str) -> Option<&'a str> {
    let (_, tag, start) = find_class(html, class)?;
    let end = close_of(html, &tag, start);
    Some(&html[start..end])
}

/// What comes after the first element with this class.
fn after_div<'a>(html: &'a str, class: &str) -> &'a str {
    let Some((_, tag, start)) = find_class(html, class) else { return html };
    let end = close_of(html, &tag, start);
    let rest = &html[end..];
    rest.split_once('>').map(|x| x.1).unwrap_or(rest)
}

/// Where the element opened before `start` closes.
fn close_of(html: &str, tag: &str, start: usize) -> usize {
    let (o, c) = (format!("<{tag}"), format!("</{tag}"));
    let mut depth = 1;
    let mut pos = start;
    loop {
        let next_o = html[pos..].find(&o).map(|x| x + pos);
        let Some(b) = html[pos..].find(&c).map(|x| x + pos) else { return html.len() };
        match next_o {
            Some(a) if a < b => {
                depth += 1;
                pos = a + o.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return b;
                }
                pos = b + c.len();
            }
        }
    }
}

/// The classes of the first element whose class list has `class`.
fn classes_of(html: &str, class: &str) -> Vec<String> {
    let Some((open, _, body)) = find_class(html, class) else { return Vec::new() };
    attr(&html[open..body], "class").unwrap_or_default().split_whitespace().map(String::from).collect()
}

fn text(html: &str) -> String {
    strip_tags(html).trim().to_string()
}

/// Text keeping its line breaks.
fn text_lines(html: &str) -> String {
    let mut s = html.to_string();
    for br in ["<br>", "<br/>", "<br />", "</p>", "</div>"] {
        s = s.replace(br, "\n");
    }
    let mut out = s.split('\n').map(strip_tags).collect::<Vec<_>>().join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string()
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// Pictures in a piece of HTML (`src`, else the first of `srcset`), leaving
/// out emoticons. Only https links.
fn img_srcs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in html.split("<img").skip(1) {
        let tag = part.split('>').next().unwrap_or_default();
        if tag.contains("class=\"emoticon\"") {
            continue;
        }
        let src = attr(tag, "src").or_else(|| attr(tag, "srcset").map(|s| s.split_whitespace().next().unwrap_or_default().to_string()));
        if let Some(s) = src.filter(|s| s.starts_with("https://")) {
            out.push(s);
        }
    }
    out
}

fn links(html: &str) -> Vec<String> {
    html.split("href=\"").skip(1).filter_map(|p| p.split('"').next()).filter(|l| l.starts_with("https://")).map(decode).collect()
}

fn links_text(html: &str) -> Vec<String> {
    html.split("<a ").skip(1).filter_map(|p| between(p, ">", "</a>")).map(text).filter(|s| !s.is_empty()).collect()
}

/// The game of the first `/app/<id>` link.
fn app_link(html: &str) -> model::AppId {
    links(html).iter().find_map(|l| l.split("/app/").nth(1)?.split(['/', '?']).next()?.parse().ok()).unwrap_or(0)
}

/// The SteamID of the first `data-miniprofile` (an account id).
fn miniprofile(html: &str) -> SteamId {
    between(html, "data-miniprofile=\"", "\"").and_then(|a| a.parse::<u64>().ok()).filter(|a| *a > 0 && *a < BASE_ID).map(|a| a + BASE_ID).unwrap_or(0)
}

/// The SteamID of the first `/profiles/<id>` link.
fn profile_id(html: &str) -> SteamId {
    links(html).iter().find_map(|l| l.split("/profiles/").nth(1)?.split(['/', '?']).next()?.parse().ok()).unwrap_or(0)
}

fn first_number(s: &str) -> u32 {
    let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.' || *c == '\u{a0}').filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests;

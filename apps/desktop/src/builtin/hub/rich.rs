//! Steam's BBCode and HTML (Workshop descriptions, guides, forum posts,
//! news) laid out natively: text with clickable links, headings, quotes,
//! lists, tables, code, spoilers, images and video thumbnails.
//!
//! Plain inline text styling follows `ui::widgets::markup`; this adds the
//! block parts (pictures, quotes) and links, which a single label can't do.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, Sense, Stroke, Ui, vec2};
use ui::{Theme, icons, widgets as w};

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Para(Vec<Span>),
    Image(String),
    /// A YouTube video id.
    Video(String),
    Quote(Vec<Block>),
    Code(String),
    Hr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// 1–3 for headings.
    pub heading: u8,
    pub spoiler: bool,
    pub dim: bool,
    pub color: Option<Color32>,
    /// Index into [`Doc::links`].
    pub link: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub blocks: Vec<Block>,
    pub links: Vec<String>,
}

/// What an opened tag changed, so its closing tag can undo it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Inline,
    Quote,
    List {
        ordered: bool,
        n: u32,
    },
    /// A `[url]x[/url]` whose target is its text.
    BareUrl(usize),
}

struct Open {
    name: String,
    prev: Style,
    kind: Kind,
}

struct Parser {
    html: bool,
    /// Finished blocks, one list per open quote (root first).
    frames: Vec<Vec<Block>>,
    para: Vec<Span>,
    style: Style,
    open: Vec<Open>,
    links: Vec<String>,
}

impl Parser {
    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let mut s = decode(s);
        if self.html {
            // HTML ignores source line breaks; `<br>` makes them.
            s = s.replace(['\r', '\n', '\t'], " ");
            while s.contains("  ") {
                s = s.replace("  ", " ");
            }
            let at_line_start = self
                .para
                .last()
                .map(|p| p.text.ends_with('\n'))
                .unwrap_or(true);
            if at_line_start {
                s = s.trim_start().to_string();
            }
            if s.is_empty() {
                return;
            }
        }
        match self.para.last_mut() {
            Some(last) if last.style == self.style => last.text.push_str(&s),
            _ => self.para.push(Span {
                text: s,
                style: self.style,
            }),
        }
    }

    fn newline(&mut self) {
        if !self.para.is_empty() {
            let style = Style {
                link: None,
                spoiler: false,
                ..self.style
            };
            match self.para.last_mut() {
                Some(last) if last.style == style => last.text.push('\n'),
                _ => self.para.push(Span {
                    text: "\n".into(),
                    style,
                }),
            }
        }
    }

    /// A line break unless we're already at the start of a line.
    fn line(&mut self) {
        if self.para.last().is_some_and(|p| !p.text.ends_with('\n')) {
            self.newline();
        }
    }

    fn flush(&mut self) {
        let mut spans = std::mem::take(&mut self.para);
        // Trim blank lines around the paragraph.
        while spans.last().is_some_and(|s| s.text.trim().is_empty()) {
            spans.pop();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        while spans.first().is_some_and(|s| s.text.trim().is_empty()) {
            spans.remove(0);
        }
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start_matches(['\n', ' ']).to_string();
        }
        if !spans.is_empty() {
            self.block(Block::Para(spans));
        }
    }

    fn block(&mut self, b: Block) {
        if let Some(f) = self.frames.last_mut() {
            f.push(b);
        }
    }

    fn push_block(&mut self, b: Block) {
        self.flush();
        self.block(b);
    }

    fn link(&mut self, url: &str) -> usize {
        self.links.push(unwrap_link(url));
        self.links.len() - 1
    }

    fn opened(&mut self, name: &str, kind: Kind) {
        self.open.push(Open {
            name: name.to_string(),
            prev: self.style,
            kind,
        });
    }

    fn close(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|o| o.name == name) else {
            return;
        };
        // Close anything left open inside it too.
        while self.open.len() > at {
            let o = self.open.pop().unwrap();
            match o.kind {
                Kind::Quote => {
                    self.flush();
                    if self.frames.len() > 1 {
                        let inner = self.frames.pop().unwrap();
                        self.block(Block::Quote(inner));
                    }
                }
                Kind::List { .. } => self.line(),
                Kind::BareUrl(i) => {
                    // `[url]https://…[/url]`: the text is the address.
                    let text: String = self
                        .para
                        .iter()
                        .rev()
                        .take_while(|s| s.style.link == Some(i))
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    if let Some(l) = self.links.get_mut(i) {
                        *l = unwrap_link(text.trim());
                    }
                }
                Kind::Inline => {}
            }
            self.style = o.prev;
        }
    }

    fn list_item(&mut self) {
        self.line();
        let bullet = match self.open.iter_mut().rev().find_map(|o| {
            if let Kind::List { ordered, n } = &mut o.kind {
                Some((ordered, n))
            } else {
                None
            }
        }) {
            Some((true, n)) => {
                *n += 1;
                format!("{n}. ")
            }
            _ => "•  ".to_string(),
        };
        let style = self.style;
        self.style = Style {
            link: None,
            ..style
        };
        self.text_raw(&bullet);
        self.style = style;
    }

    fn text_raw(&mut self, s: &str) {
        match self.para.last_mut() {
            Some(last) if last.style == self.style => last.text.push_str(s),
            _ => self.para.push(Span {
                text: s.to_string(),
                style: self.style,
            }),
        }
    }
}

/// Undo Steam's link filter (`/linkfilter/?u=…`) so links show their target.
pub fn unwrap_link(url: &str) -> String {
    let url = decode(url.trim());
    for key in ["linkfilter/?u=", "linkfilter/?url="] {
        if let Some(i) = url.find(key) {
            let target = &url[i + key.len()..];
            return percent_decode(target.split('&').next().unwrap_or(target));
        }
    }
    url
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode(s: &str) -> String {
    client::hub::decode(s)
}

/// An HTML attribute's value.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| i + from) {
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = &tag[i + name.len()..];
        let rest_trim = rest.trim_start();
        if before_ok && rest_trim.starts_with('=') {
            let v = rest_trim[1..].trim_start();
            let (q, body) = match v.chars().next() {
                Some(c @ ('"' | '\'')) => (Some(c), &v[1..]),
                _ => (None, v),
            };
            let end = match q {
                Some(q) => body.find(q).unwrap_or(body.len()),
                None => body.find([' ', '>']).unwrap_or(body.len()),
            };
            return Some(body[..end].to_string());
        }
        from = i + name.len();
    }
    None
}

fn youtube_id(s: &str) -> Option<String> {
    for key in [
        "youtube.com/embed/",
        "youtube.com/watch?v=",
        "youtu.be/",
        "youtube-nocookie.com/embed/",
    ] {
        if let Some(i) = s.find(key) {
            let id: String = s[i + key.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if id.len() >= 6 {
                return Some(id);
            }
        }
    }
    None
}

fn parse_color(v: &str) -> Option<Color32> {
    let v = v
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim_start_matches('#');
    let n = u32::from_str_radix(v.get(..6)?, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Parse Steam markup: BBCode (`[b]`, `[url=…]`, `[img]`, `[quote]`, `[list]`,
/// `[table]`, `[previewyoutube]`) and the HTML Steam renders it to.
pub fn parse(src: &str) -> Doc {
    let lower_src = src.to_ascii_lowercase();
    let html = [
        "<br",
        "<div",
        "<p>",
        "<a ",
        "<img",
        "<span",
        "<blockquote",
        "<ul",
    ]
    .iter()
    .any(|t| lower_src.contains(t));
    let mut p = Parser {
        html,
        frames: vec![Vec::new()],
        para: Vec::new(),
        style: Style::default(),
        open: Vec::new(),
        links: Vec::new(),
    };
    let mut rest = src;
    while !rest.is_empty() {
        let Some(at) = rest.find(['[', '<']) else {
            p.text(rest);
            break;
        };
        p.text(&rest[..at]);
        let bracket = rest.as_bytes()[at] == b'[';
        let close_ch = if bracket { ']' } else { '>' };
        let Some(end) = rest[at..].find(close_ch).map(|e| e + at) else {
            p.text(&rest[at..]);
            break;
        };
        let tag = &rest[at + 1..end];
        let after = &rest[end + 1..];
        let lower = tag.to_ascii_lowercase();
        let closing = lower.starts_with('/');
        let name: String = lower
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '*')
            .collect();
        if bracket {
            match handle_bbcode(&mut p, &name, tag, closing, after) {
                Some(skip) => rest = &after[skip..],
                None => {
                    // Not a tag: keep the bracket as text.
                    p.text(&rest[at..at + 1]);
                    rest = &rest[at + 1..];
                }
            }
        } else {
            if name.is_empty() && !lower.starts_with("!--") {
                p.text(&rest[at..at + 1]);
                rest = &rest[at + 1..];
                continue;
            }
            rest = &after[handle_html(&mut p, &name, tag, closing, after)..];
        }
    }
    p.flush();
    // Close quotes left open.
    while p.frames.len() > 1 {
        let inner = p.frames.pop().unwrap();
        p.block(Block::Quote(inner));
    }
    Doc {
        blocks: p.frames.pop().unwrap_or_default(),
        links: p.links,
    }
}

/// Handles one BBCode tag; returns how much of `after` it used, or `None`
/// if it isn't a tag.
fn handle_bbcode(
    p: &mut Parser,
    name: &str,
    tag: &str,
    closing: bool,
    after: &str,
) -> Option<usize> {
    let value = tag
        .split_once('=')
        .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_string());
    // Tags whose content is taken as is.
    let raw = |end: &str| -> (String, usize) {
        let lower = after.to_ascii_lowercase();
        match lower.find(end) {
            Some(i) => (after[..i].to_string(), i + end.len()),
            None => (after.to_string(), after.len()),
        }
    };
    if closing {
        match name {
            "b" | "i" | "u" | "s" | "strike" | "url" | "color" | "spoiler" | "quote" | "list"
            | "olist" | "h1" | "h2" | "h3" | "th" => {
                if matches!(name, "h1" | "h2" | "h3") {
                    p.close(name);
                    p.newline();
                } else {
                    p.close(name);
                }
            }
            "tr" => p.line(),
            "td" | "table" | "previewimg" | "previewyoutube" | "img" | "code" | "noparse"
            | "hr" | "*" => {
                if name == "table" {
                    p.line();
                }
            }
            _ => return None,
        }
        return Some(0);
    }
    match name {
        "b" | "i" | "u" | "s" | "strike" | "spoiler" | "h1" | "h2" | "h3" | "th" => {
            if matches!(name, "h1" | "h2" | "h3") {
                p.line();
            }
            p.opened(name, Kind::Inline);
            match name {
                "b" | "th" => p.style.bold = true,
                "i" => p.style.italic = true,
                "u" => p.style.underline = true,
                "s" | "strike" => p.style.strike = true,
                "spoiler" => p.style.spoiler = true,
                h => p.style.heading = h[1..].parse().unwrap_or(1),
            }
        }
        "color" => {
            p.opened(name, Kind::Inline);
            p.style.color = value.as_deref().and_then(parse_color).or(p.style.color);
        }
        "url" => match value.filter(|v| !v.is_empty()) {
            Some(v) => {
                let i = p.link(&v);
                p.opened(name, Kind::Inline);
                p.style.link = Some(i);
            }
            None => {
                let i = p.link("");
                p.opened(name, Kind::BareUrl(i));
                p.style.link = Some(i);
            }
        },
        "img" => {
            let (url, used) = raw("[/img]");
            let url = url.trim().to_string();
            if url.starts_with("http") {
                p.push_block(Block::Image(url));
            }
            return Some(used);
        }
        "previewyoutube" => {
            let id = value
                .unwrap_or_default()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_string();
            if !id.is_empty() {
                p.push_block(Block::Video(id));
            }
        }
        // Inline pictures by preview id: the item's pictures show above instead.
        "previewimg" => {
            let (_, used) = raw("[/previewimg]");
            return Some(used);
        }
        "code" => {
            let (code, used) = raw("[/code]");
            p.push_block(Block::Code(decode(code.trim_matches('\n'))));
            return Some(used);
        }
        "noparse" => {
            let (text, used) = raw("[/noparse]");
            p.text(&text);
            return Some(used);
        }
        "quote" => {
            p.flush();
            p.frames.push(Vec::new());
            p.opened(name, Kind::Quote);
            if let Some(author) = value
                .as_deref()
                .map(|v| v.split(';').next().unwrap_or(v).trim())
                .filter(|a| !a.is_empty())
            {
                let prev = p.style;
                p.style = Style {
                    italic: true,
                    dim: true,
                    ..Style::default()
                };
                p.text_raw(&i18n::t!("Originally posted by {name}:", name = author));
                p.newline();
                p.style = prev;
            }
        }
        "list" | "olist" => {
            p.line();
            p.opened(
                name,
                Kind::List {
                    ordered: name == "olist",
                    n: 0,
                },
            );
        }
        "*" => p.list_item(),
        "hr" => p.push_block(Block::Hr),
        "table" => p.line(),
        "tr" => p.line(),
        "td" => {
            if p.para.last().is_some_and(|s| !s.text.ends_with('\n')) {
                p.text_raw("   ·   ");
            }
        }
        "br" => p.newline(),
        _ => return None,
    }
    Some(0)
}

/// Handles one HTML tag; returns how much of `after` it used.
fn handle_html(p: &mut Parser, name: &str, tag: &str, closing: bool, after: &str) -> usize {
    let class = attr(tag, "class").unwrap_or_default();
    let has = |c: &str| class.split_whitespace().any(|x| x == c);
    if closing {
        match name {
            "div" | "span" | "blockquote" | "a" | "b" | "strong" | "i" | "em" | "u" | "s"
            | "strike" | "del" | "font" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "th" => {
                let heading = p.style.heading > 0;
                p.close(name);
                if name == "div" || heading || matches!(name, "h1" | "h2" | "h3" | "h4") {
                    p.line();
                }
            }
            "p" => {
                p.line();
                p.newline();
            }
            "tr" | "table" => p.line(),
            _ => {}
        }
        return 0;
    }
    match name {
        "br" => p.newline(),
        "p" => p.line(),
        "hr" => p.push_block(Block::Hr),
        "script" | "style" => {
            let end = format!("</{name}>");
            return after
                .to_ascii_lowercase()
                .find(&end)
                .map(|i| i + end.len())
                .unwrap_or(after.len());
        }
        "img" => {
            let src = attr(tag, "src").unwrap_or_default();
            if has("emoticon") || src.contains("/emoticon/") {
                // Small inline emoticons: their name as text.
                if let Some(alt) = attr(tag, "alt") {
                    p.text(&alt);
                }
            } else if src.starts_with("http") {
                p.push_block(Block::Image(src));
            }
        }
        "iframe" => {
            if let Some(id) = attr(tag, "src").as_deref().and_then(youtube_id) {
                p.push_block(Block::Video(id));
            }
            return after
                .to_ascii_lowercase()
                .find("</iframe>")
                .map(|i| i + 9)
                .unwrap_or(0);
        }
        "a" => {
            let href = attr(tag, "href").unwrap_or_default();
            // Steam wraps every picture in a link to itself; the picture is enough.
            let wraps_image = has("modalContentLink")
                || after.trim_start().to_ascii_lowercase().starts_with("<img");
            p.opened(name, Kind::Inline);
            if !href.is_empty() && !href.starts_with("javascript") && !wraps_image {
                let i = p.link(&href);
                p.style.link = Some(i);
            }
        }
        "b" | "strong" | "th" => {
            p.opened(name, Kind::Inline);
            p.style.bold = true;
        }
        "i" | "em" => {
            p.opened(name, Kind::Inline);
            p.style.italic = true;
        }
        "u" => {
            p.opened(name, Kind::Inline);
            p.style.underline = true;
        }
        "s" | "strike" | "del" => {
            p.opened(name, Kind::Inline);
            p.style.strike = true;
        }
        "font" => {
            p.opened(name, Kind::Inline);
            p.style.color = attr(tag, "color")
                .as_deref()
                .and_then(parse_color)
                .or(p.style.color);
        }
        "h1" | "h2" | "h3" | "h4" => {
            p.line();
            p.opened(name, Kind::Inline);
            p.style.heading = name[1..].parse::<u8>().unwrap_or(1).min(3);
        }
        "blockquote" => {
            p.flush();
            p.frames.push(Vec::new());
            p.opened(name, Kind::Quote);
        }
        "ul" | "ol" => {
            p.line();
            p.opened(
                name,
                Kind::List {
                    ordered: name == "ol",
                    n: 0,
                },
            );
        }
        "li" => p.list_item(),
        "tr" => p.line(),
        "td" => {
            if p.para.last().is_some_and(|s| !s.text.ends_with('\n')) {
                p.text_raw("   ·   ");
            }
        }
        "div" | "span" => {
            if has("bb_code") || has("codeblock") {
                // Code keeps its own lines.
                let end = after
                    .to_ascii_lowercase()
                    .find("</div>")
                    .unwrap_or(after.len());
                let code = after[..end].replace("<br>", "\n").replace("<br/>", "\n");
                p.push_block(Block::Code(client::hub::decode(&strip(&code))));
                return (end + 6).min(after.len());
            }
            if class.contains("YouTubeVideo") || class.contains("youtube") {
                if let Some(id) = attr(tag, "data-youtube")
                    .or_else(|| {
                        attr(tag, "id").map(|i| i.trim_start_matches("youtubeEmbed_").to_string())
                    })
                    .filter(|i| i.len() >= 6)
                {
                    p.push_block(Block::Video(id));
                }
            }
            if name == "div" {
                p.line();
            }
            if has("bb_blockquote") || has("quote") {
                p.flush();
                p.frames.push(Vec::new());
                p.opened(name, Kind::Quote);
                return 0;
            }
            p.opened(name, Kind::Inline);
            if let Some(h) = ["bb_h1", "bb_h2", "bb_h3", "subSectionTitle"]
                .iter()
                .position(|c| has(c))
            {
                p.style.heading = (h as u8 + 1).min(3);
            }
            if has("bb_spoiler") {
                p.style.spoiler = true;
            }
            if has("bb_quoteauthor") {
                p.style.italic = true;
                p.style.dim = true;
            }
            if has("bb_strike") {
                p.style.strike = true;
            }
            if let Some(c) = attr(tag, "style").and_then(|s| {
                s.split(';').find_map(|r| {
                    r.trim()
                        .strip_prefix("color:")
                        .map(|v| v.trim().to_string())
                })
            }) {
                p.style.color = parse_color(&c).or(p.style.color);
            }
            if has("bb_hr") {
                p.push_block(Block::Hr);
            }
        }
        _ => {}
    }
    0
}

fn strip(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ---------- layout ----------

fn job(
    t: &Theme,
    spans: &[Span],
    size: f32,
    color: Color32,
    width: f32,
    reveal: bool,
) -> (LayoutJob, Vec<Option<usize>>) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut links = Vec::new();
    for s in spans {
        let st = s.style;
        let fs = match st.heading {
            1 => size * 1.45,
            2 => size * 1.25,
            3 => size * 1.12,
            _ => size,
        };
        let mut c = st.color.unwrap_or(if st.dim { t.text_dim } else { color });
        if st.link.is_some() {
            c = t.accent;
        }
        let mut f = TextFormat {
            font_id: if st.bold || st.heading > 0 {
                w::font_semibold(fs)
            } else {
                w::font(fs)
            },
            color: c,
            italics: st.italic,
            ..Default::default()
        };
        if st.underline {
            f.underline = Stroke::new(1.0, c);
        }
        if st.strike {
            f.strikethrough = Stroke::new(1.0, c);
        }
        if st.spoiler && !reveal {
            f.color = t.text_faint.gamma_multiply(0.0);
            f.background = t.raised;
        } else if st.spoiler {
            f.background = t.raised;
        }
        if st.heading > 0 {
            f.line_height = Some(fs * 1.6);
        }
        job.append(&s.text, 0.0, f);
        links.push(st.link);
    }
    (job, links)
}

/// Draw a document. Returns a link the user clicked (to open natively when
/// it's a Steam page, in the browser otherwise) or a picture to enlarge
/// (prefixed with `image:`).
pub fn show(ui: &mut Ui, t: &Theme, doc: &Doc, size: f32) -> Option<String> {
    let mut clicked = None;
    show_blocks(ui, t, &doc.blocks, &doc.links, size, &mut clicked, 0);
    clicked
}

fn show_blocks(
    ui: &mut Ui,
    t: &Theme,
    blocks: &[Block],
    links: &[String],
    size: f32,
    clicked: &mut Option<String>,
    depth: usize,
) {
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            ui.add_space(size * 0.55);
        }
        match b {
            Block::Para(spans) => {
                let width = ui.available_width();
                let id = ui.next_auto_id();
                let reveal = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                let (job, link_of) = job(t, spans, size, t.text, width, reveal);
                let sections: Vec<std::ops::Range<usize>> = job
                    .sections
                    .iter()
                    .map(|s| s.byte_range.start.0..s.byte_range.end.0)
                    .collect();
                let text = job.text.clone();
                let galley = ui.painter().layout_job(job);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(width, galley.size().y), Sense::click());
                // Which link (if any) is under the pointer.
                let hovered_link = resp.hover_pos().and_then(|pos| {
                    let c = galley.cursor_from_pos(pos - rect.min);
                    let byte = text.char_indices().nth(c.index.0).map(|(b, _)| b)?;
                    let s = sections.iter().position(|r| r.contains(&byte))?;
                    link_of.get(s).copied().flatten()
                });
                let has_spoiler = spans.iter().any(|s| s.style.spoiler);
                ui.painter().galley(rect.min, galley, t.text);
                if let Some(l) = hovered_link {
                    let url = links.get(l).cloned().unwrap_or_default();
                    let resp = resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(&url);
                    if resp.clicked() && !url.is_empty() {
                        *clicked = Some(url);
                    }
                } else if has_spoiler && resp.clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(id, !reveal));
                } else if has_spoiler {
                    resp.on_hover_text(i18n::t!("Spoiler: click to show"));
                }
            }
            Block::Image(url) => {
                let max = ui.available_width().min(900.0);
                let img = egui::Image::new(url.clone())
                    .max_width(max)
                    .max_height(700.0)
                    .fit_to_original_size(1.0)
                    .corner_radius(CornerRadius::same(4))
                    .sense(Sense::click());
                let r = ui.add(img);
                if r.rect.height() < 4.0 {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(max.min(480.0), 220.0), Sense::hover());
                    w::image_or_skeleton(ui, t, rect, url, CornerRadius::same(4));
                }
                if r.on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                    *clicked = Some(format!("image:{url}"));
                }
            }
            Block::Video(id) => {
                let wdt = ui.available_width().min(640.0);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(wdt, wdt * 9.0 / 16.0), Sense::click());
                w::image_or_skeleton(
                    ui,
                    t,
                    rect,
                    &format!("https://img.youtube.com/vi/{id}/hqdefault.jpg"),
                    CornerRadius::same(4),
                );
                let hover = resp.hovered();
                ui.painter().circle_filled(
                    rect.center(),
                    30.0,
                    if hover {
                        t.accent
                    } else {
                        Color32::from_black_alpha(170)
                    },
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icons::PLAY,
                    w::font(26.0),
                    Color32::WHITE,
                );
                if resp
                    .on_hover_text(i18n::t!("Watch on YouTube"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    *clicked = Some(format!("https://www.youtube.com/watch?v={id}"));
                }
            }
            Block::Quote(inner) => {
                egui::Frame::new()
                    .fill(t.raised.gamma_multiply(0.7))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 10,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let top = ui.min_rect().top();
                        if depth < 6 {
                            show_blocks(ui, t, inner, links, size * 0.95, clicked, depth + 1);
                        }
                        let left = ui.min_rect().left() - 10.0;
                        ui.painter().vline(
                            left,
                            top..=ui.min_rect().bottom(),
                            Stroke::new(3.0, t.border),
                        );
                    });
            }
            Block::Code(code) => {
                egui::Frame::new()
                    .fill(t.raised)
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(code)
                                    .monospace()
                                    .size(size * 0.9)
                                    .color(t.text),
                            )
                            .wrap(),
                        );
                    });
            }
            Block::Hr => {
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                ui.painter()
                    .hline(r.x_range(), r.center().y, Stroke::new(1.0, t.border));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &Doc) -> String {
        doc.blocks
            .iter()
            .map(|b| match b {
                Block::Para(s) => s.iter().map(|s| s.text.as_str()).collect::<String>(),
                Block::Image(u) => format!("[img {u}]"),
                Block::Video(v) => format!("[video {v}]"),
                Block::Quote(q) => format!(
                    "[quote {}]",
                    text(&Doc {
                        blocks: q.clone(),
                        links: vec![]
                    })
                ),
                Block::Code(c) => format!("[code {c}]"),
                Block::Hr => "[hr]".into(),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[test]
    fn bbcode() {
        let d = parse(
            "[h1]Install[/h1]Put it in [b]Mods[/b].\n[img]https://i/1.png[/img]\n[list][*]one[*]two[/list][url=https://example.com]site[/url] [url]https://x.y[/url]\n[quote=Ferris;123]hi[/quote][previewyoutube=abcdefgh;full][/previewyoutube]",
        );
        assert_eq!(
            text(&d),
            "Install\nPut it in Mods. | [img https://i/1.png] | •  one\n•  two\nsite https://x.y | [quote Originally posted by Ferris:\nhi] | [video abcdefgh]"
        );
        assert_eq!(
            d.links,
            vec!["https://example.com".to_string(), "https://x.y".to_string()]
        );
        let Block::Para(s) = &d.blocks[0] else {
            panic!()
        };
        assert_eq!(s[0].style.heading, 1);
        assert!(s.iter().any(|s| s.text == "Mods" && s.style.bold));
        // Brackets that aren't tags stay.
        assert_eq!(text(&parse("[x] and [y]")), "[x] and [y]");
    }

    #[test]
    fn html() {
        let d = parse(
            r#"Hello <b>there</b><br>Line two<div class="bb_h1">Title</div><blockquote class="bb_blockquote"><div class="bb_quoteauthor">Originally posted by <b>A</b>:</div>quoted</blockquote>after <a href="https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwiki.example%2Fa" class="bb_link">wiki</a><a href="https://img/full.png" class="modalContentLink"><img src="https://img/full.png" class="sharedFilePreviewImage"></a><ul class="bb_ul"><li>a</li><li>b</li></ul><span class="bb_spoiler"><span>secret</span></span>"#,
        );
        assert_eq!(
            text(&d),
            "Hello there\nLine two\nTitle | [quote Originally posted by A:\nquoted] | after wiki | [img https://img/full.png] | •  a\n•  b\nsecret"
        );
        assert_eq!(d.links, vec!["https://wiki.example/a".to_string()]);
        let Block::Para(s) = d.blocks.last().unwrap() else {
            panic!()
        };
        assert!(s.iter().any(|s| s.text == "secret" && s.style.spoiler));
    }

    #[test]
    fn code_and_entities() {
        let d = parse("[code]let x = 1;\nlet y = &amp;x;[/code]Tom&#39;s");
        assert_eq!(text(&d), "[code let x = 1;\nlet y = &x;] | Tom's");
    }
}

//! Turns the small subset of HTML used in store descriptions into blocks the
//! UI can lay out natively (headings, paragraphs, bullet points, images).

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(String),
    /// Runs of (text, bold).
    Paragraph(Vec<(String, bool)>),
    Bullet(Vec<(String, bool)>),
    Image(String),
    /// A YouTube video id.
    Video(String),
}

/// Plain text with tags removed and entities decoded.
pub fn text(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode(&out)
        .chars()
        .filter(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{2028}' | '\u{2029}') && !c.is_control() || *c == ' ' || *c == '\n')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&reg;", "®")
        .replace("&trade;", "™")
        .replace("&copy;", "©")
        .replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &tag[i..];
    Some(rest[..rest.find('"')?].to_string())
}

pub fn valid_http_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.chars().any(|c| c.is_control()) {
        return None;
    }
    let rest = if s.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        &s[8..]
    } else if s.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")) {
        &s[7..]
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('\\') {
        return None;
    }
    let parsed = reqwest::Url::parse(s).ok()?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    if parsed.host_str().is_none_or(|h| h.is_empty()) {
        return None;
    }
    Some(s.to_string())
}

pub fn valid_youtube_id(raw: &str) -> Option<&str> {
    let id = raw.trim();
    (!id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(id)
}

pub fn parse(html: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut runs: Vec<(String, bool)> = Vec::new();
    let mut bold = 0i32;
    let mut in_heading = false;
    let mut in_li = false;
    let mut buf = String::new();

    fn push_text(runs: &mut Vec<(String, bool)>, buf: &mut String, bold: bool) {
        let t = decode(buf);
        buf.clear();
        if t.trim().is_empty() && runs.is_empty() {
            return;
        }
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some((prev, b)) if *b == bold => {
                prev.push(' ');
                prev.push_str(&t);
            }
            _ => runs.push((t, bold)),
        }
    }
    fn flush(blocks: &mut Vec<Block>, runs: &mut Vec<(String, bool)>, li: bool) {
        if runs.iter().any(|(t, _)| !t.trim().is_empty()) {
            let r = std::mem::take(runs);
            blocks.push(if li { Block::Bullet(r) } else { Block::Paragraph(r) });
        }
        runs.clear();
    }

    let mut rest = html;
    while !rest.is_empty() {
        if let Some(start) = rest.find('<') {
            buf.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('>') else { break };
            let tag = &rest[start + 1..start + end];
            rest = &rest[start + end + 1..];
            let lower = tag.to_ascii_lowercase();
            let name = lower.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
            let closing = lower.starts_with('/');
            match name {
                "b" | "strong" => {
                    push_text(&mut runs, &mut buf, bold > 0);
                    bold += if closing { -1 } else { 1 };
                    bold = bold.max(0);
                }
                "h1" | "h2" | "h3" | "h4" => {
                    if closing {
                        let t = decode(&buf).trim().to_string();
                        buf.clear();
                        if !t.is_empty() {
                            blocks.push(Block::Heading(t));
                        }
                        in_heading = false;
                    } else {
                        push_text(&mut runs, &mut buf, bold > 0);
                        flush(&mut blocks, &mut runs, in_li);
                        in_heading = true;
                    }
                }
                "li" => {
                    push_text(&mut runs, &mut buf, bold > 0);
                    flush(&mut blocks, &mut runs, in_li);
                    in_li = !closing;
                }
                "br" | "p" | "ul" | "ol" | "div" if !in_heading => {
                    push_text(&mut runs, &mut buf, bold > 0);
                    flush(&mut blocks, &mut runs, in_li);
                }
                "img" => {
                    push_text(&mut runs, &mut buf, bold > 0);
                    flush(&mut blocks, &mut runs, in_li);
                    if let Some(src) = attr(tag, "src").map(|s| decode(&s)).and_then(|s| valid_http_url(&s)) {
                        blocks.push(Block::Image(src));
                    }
                }
                _ => {}
            }
        } else {
            buf.push_str(rest);
            break;
        }
    }
    push_text(&mut runs, &mut buf, bold > 0);
    flush(&mut blocks, &mut runs, in_li);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_store_html() {
        let b = parse(r#"<h2 class="x">About</h2>Fight <strong>gods</strong> &amp; more.<br><img src="https://a/b.gif" /><ul><li><strong>OS:</strong> Windows 10</li></ul>"#);
        assert_eq!(b[0], Block::Heading("About".into()));
        assert_eq!(b[1], Block::Paragraph(vec![("Fight".into(), false), ("gods".into(), true), ("& more.".into(), false)]));
        assert_eq!(b[2], Block::Image("https://a/b.gif".into()));
        assert_eq!(b[3], Block::Bullet(vec![("OS:".into(), true), ("Windows 10".into(), false)]));
    }

    #[test]
    fn rejects_non_http_img_sources() {
        let b = parse(
            r#"<img src="file://attacker/share/pwn.png" /><img src="file:///etc/passwd" /><img src="javascript:alert(1)" /><img src="//attacker/p.png" /><img src="https://" /><img src="http:\\attacker\share\a.png" /><img src="http://ok.example/a.png" />"#,
        );
        assert_eq!(b, vec![Block::Image("http://ok.example/a.png".into())]);
    }
}

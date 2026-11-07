//! HLS playlists (RFC 8216): the master list of quality variants and the
//! media playlists of segments. Steam's trailers use fMP4 segments with a
//! separate audio rendition.

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub bandwidth: u64,
    /// URL of the separate audio playlist, if any.
    pub audio: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Media {
    pub init: Option<String>,
    /// (url, duration in seconds)
    pub segments: Vec<(String, f64)>,
}

/// Resolve a playlist URI against the playlist's own URL.
pub fn resolve(base: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_string();
    }
    if let Ok(b) = reqwest::Url::parse(base) {
        if let Ok(u) = b.join(uri) {
            return u.to_string();
        }
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{}", &base[..i], uri),
        None => uri.to_string(),
    }
}

/// `KEY=value,KEY="quoted, value"` attribute lists.
fn attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        rest = &rest[eq + 1..];
        let value;
        if let Some(r) = rest.strip_prefix('"') {
            let end = r.find('"').unwrap_or(r.len());
            value = r[..end].to_string();
            rest = r.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].to_string();
            rest = &rest[end..];
        }
        rest = rest.strip_prefix(',').unwrap_or(rest);
        out.push((key, value));
    }
    out
}

fn get<'a>(a: &'a [(String, String)], k: &str) -> Option<&'a str> {
    a.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
}

pub fn is_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

/// Variants of a master playlist, smallest first.
pub fn parse_master(text: &str, base: &str) -> Vec<Variant> {
    let mut audio_groups: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(a) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attrs(a);
            if get(&a, "TYPE") == Some("AUDIO") {
                if let (Some(g), Some(u)) = (get(&a, "GROUP-ID"), get(&a, "URI")) {
                    let default = get(&a, "DEFAULT") == Some("YES");
                    if default || !audio_groups.iter().any(|(gg, _)| gg == g) {
                        audio_groups.retain(|(gg, _)| gg != g);
                        audio_groups.push((g.to_string(), resolve(base, u)));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in text.lines().map(str::trim) {
        if let Some(a) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attrs(a));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(a) = pending.take() {
                let (width, height) = get(&a, "RESOLUTION")
                    .and_then(|r| r.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or((0, 0));
                let audio = get(&a, "AUDIO").and_then(|g| audio_groups.iter().find(|(gg, _)| gg == g)).map(|(_, u)| u.clone());
                out.push(Variant { url: resolve(base, line), width, height, bandwidth: get(&a, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0), audio });
            }
        }
    }
    out.sort_by_key(|v| (v.height, v.bandwidth));
    out
}

pub fn parse_media(text: &str, base: &str) -> Media {
    let mut m = Media::default();
    let mut dur = None;
    for line in text.lines().map(str::trim) {
        if let Some(a) = line.strip_prefix("#EXT-X-MAP:") {
            m.init = get(&attrs(a), "URI").map(|u| resolve(base, u));
        } else if let Some(d) = line.strip_prefix("#EXTINF:") {
            dur = d.split(',').next().and_then(|d| d.trim().parse::<f64>().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            m.segments.push((resolve(base, line), dur.take().unwrap_or(0.0)));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="Default",AUTOSELECT=YES,DEFAULT=YES,URI="hls_264_4_audio.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=5800000,CODECS="avc1.640029,mp4a.40.2",RESOLUTION=1920x1080,FRAME-RATE=30,AUDIO="audio"
hls_264_0_video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1000000,CODECS="avc1.640029,mp4a.40.2",RESOLUTION=640x360,FRAME-RATE=30,AUDIO="audio"
hls_264_3_video.m3u8
"#;

    #[test]
    fn master() {
        let v = parse_master(MASTER, "https://cdn/x/1/hls_264_master.m3u8");
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].width, v[0].height, v[0].bandwidth), (640, 360, 1_000_000));
        assert_eq!(v[0].url, "https://cdn/x/1/hls_264_3_video.m3u8");
        assert_eq!(v[1].audio.as_deref(), Some("https://cdn/x/1/hls_264_4_audio.m3u8"));
    }

    #[test]
    fn media() {
        let m = parse_media("#EXTM3U\n#EXT-X-MAP:URI=\"dash_h264/init-stream1.m4s\"\n#EXTINF:3,\ndash_h264/a.m4s\n#EXTINF:1.5\ndash_h264/b.m4s\n#EXT-X-ENDLIST\n", "https://cdn/x/v.m3u8");
        assert_eq!(m.init.as_deref(), Some("https://cdn/x/dash_h264/init-stream1.m4s"));
        assert_eq!(m.segments, vec![("https://cdn/x/dash_h264/a.m4s".to_string(), 3.0), ("https://cdn/x/dash_h264/b.m4s".to_string(), 1.5)]);
    }
}

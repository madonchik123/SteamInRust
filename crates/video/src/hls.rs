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
    /// The number of the first segment listed (`#EXT-X-MEDIA-SEQUENCE`). A
    /// live playlist's window moves forward; numbers stay with their segment.
    pub sequence: u64,
    /// The longest segment, in seconds (`#EXT-X-TARGETDURATION`), 0 if unsaid.
    pub target: f64,
    /// No more segments will come (`#EXT-X-ENDLIST`).
    pub ended: bool,
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

const MAX_VARIANTS: usize = 64;
const MAX_SEGMENTS: usize = 5_000;
const MIN_EXTINF: f64 = 0.05;

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
                if out.len() >= MAX_VARIANTS {
                    break;
                }
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
        } else if let Some(n) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            m.sequence = n.trim().parse().unwrap_or(0);
        } else if let Some(d) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            m.target = d.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0);
        } else if line == "#EXT-X-ENDLIST" {
            m.ended = true;
        } else if let Some(d) = line.strip_prefix("#EXTINF:") {
            dur = d.split(',').next().and_then(|d| d.trim().parse::<f64>().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            if m.segments.len() >= MAX_SEGMENTS {
                break;
            }
            let d = dur.take().filter(|v| v.is_finite()).map_or(MIN_EXTINF, |v| v.max(MIN_EXTINF));
            m.segments.push((resolve(base, line), d));
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
        assert!(m.ended);
    }

    /// The shape of a Steam broadcast's playlist: absolute links with a query,
    /// a moving window, no end marker while it's live.
    #[test]
    fn live_playlist() {
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:148775\n#EXT-X-MAP:URI=\"https://cache.example/broadcast/1/2/segment/video/1/init/?broadcast_origin=o\"\n#EXTINF:2.000000,\nhttps://cache.example/broadcast/1/2/segment/video/1/148775/?broadcast_origin=o\n#EXTINF:2.000000,\nhttps://cache.example/broadcast/1/2/segment/video/1/148776/?broadcast_origin=o\n";
        let m = parse_media(text, "https://cache.example/broadcast/1/2/hls_manifest/0/h/6635000/video.m3u8?broadcast_origin=o");
        assert_eq!((m.sequence, m.target, m.ended), (148775, 2.0, false));
        assert_eq!(m.init.as_deref(), Some("https://cache.example/broadcast/1/2/segment/video/1/init/?broadcast_origin=o"));
        assert_eq!(m.segments.len(), 2);
        assert!(m.segments[1].0.contains("/148776/"));
        assert!(parse_media(&format!("{text}#EXT-X-ENDLIST\n"), "https://cache.example/v.m3u8").ended);
    }

    #[test]
    fn broadcast_master_keeps_every_quality_and_the_audio() {
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"Default\",AUTOSELECT=YES,DEFAULT=YES,URI=\"https://c/125000/audio.m3u8?o=1\"\n#EXT-X-STREAM-INF:BANDWIDTH=6760000,CODECS=\"avc1.4d402a,mp4a.40.2\",RESOLUTION=1920x1080,FRAME-RATE=60,AUDIO=\"aac\"\nhttps://c/6635000/video.m3u8?o=1\n#EXT-X-STREAM-INF:BANDWIDTH=1625000,CODECS=\"avc1.64001f,mp4a.40.2\",RESOLUTION=854x480,FRAME-RATE=30,AUDIO=\"aac\"\nhttps://c/1500000/video.m3u8?o=1\n";
        let v = parse_master(text, "https://c/master.m3u8?o=1");
        assert_eq!(v.iter().map(|x| x.height).collect::<Vec<_>>(), [480, 1080]);
        assert!(v.iter().all(|x| x.audio.as_deref() == Some("https://c/125000/audio.m3u8?o=1")));
    }

    #[test]
    fn clamps_extinf_and_caps_segments() {
        let mut playlist = String::from("#EXTM3U\n#EXTINF:-5,\nseg0.m4s\n#EXTINF:0,\nseg1.m4s\n#EXTINF:NaN,\nseg2.m4s\n#EXTINF:inf,\nseg3.m4s\nseg4.m4s\n");
        for i in 5..5_100 {
            playlist.push_str(&format!("#EXTINF:1.0,\nseg{i}.m4s\n"));
        }
        let m = parse_media(&playlist, "https://cdn/x/v.m3u8");
        assert_eq!(m.segments.len(), 5_000);
        for (_, d) in &m.segments {
            assert!(d.is_finite() && *d >= 0.05, "bad segment duration {d}");
        }
    }
}

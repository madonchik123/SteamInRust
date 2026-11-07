//! Download regions: every Steam content-server region, with a measured
//! connection time from this PC, sorted fastest first.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Steam region ids to look through. Each answers with servers in its own
/// region (or the nearest one), so together they cover every region.
const CELLS: std::ops::RangeInclusive<u32> = 1..=230;

/// Airport-style codes in server names -> city.
fn city(code: &str) -> String {
    let name = match code {
        "ams" => "Amsterdam", "atl" => "Atlanta", "bom" => "Mumbai", "can" => "Guangzhou", "cdg" | "par" => "Paris",
        "chi" | "ord" => "Chicago", "dfw" => "Dallas", "dxb" => "Dubai", "eze" => "Buenos Aires", "fra" => "Frankfurt",
        "gru" => "São Paulo", "hel" => "Helsinki", "hkg" => "Hong Kong", "iad" => "Washington, D.C.", "icn" | "sel" => "Seoul",
        "jnb" => "Johannesburg", "lax" => "Los Angeles", "lhr" | "lon" => "London", "lim" => "Lima", "mad" => "Madrid",
        "man" => "Manchester", "mel" => "Melbourne", "mia" => "Miami", "mil" | "mxp" => "Milan", "mow" | "svo" | "dme" => "Moscow",
        "nrt" | "tyo" | "hnd" => "Tokyo", "nyc" | "jfk" | "ewr" => "New York", "osa" | "kix" => "Osaka", "per" => "Perth",
        "prg" => "Prague", "scl" => "Santiago", "sea" => "Seattle", "sgp" | "sin" => "Singapore", "sto" | "arn" => "Stockholm",
        "syd" => "Sydney", "tpe" => "Taipei", "vie" => "Vienna", "waw" => "Warsaw", "led" => "Saint Petersburg", "ala" => "Almaty",
        "ist" => "Istanbul", "bkk" => "Bangkok", "kul" => "Kuala Lumpur", "mnl" => "Manila", "sjc" => "San Jose", "den" => "Denver",
        "yyz" => "Toronto", "yvr" => "Vancouver", "bog" => "Bogotá", "mex" => "Mexico City", "tlv" => "Tel Aviv", "cph" => "Copenhagen",
        "bru" => "Brussels", "zrh" => "Zurich", "dub" => "Dublin", "osl" => "Oslo", "lis" => "Lisbon", "bud" => "Budapest",
        "buh" | "otp" => "Bucharest", "kiv" => "Chișinău", "kbp" | "iev" => "Kyiv", "tas" => "Tashkent", "nqz" | "tse" => "Astana",
        "akl" => "Auckland", "bne" => "Brisbane", "hyd" => "Hyderabad", "maa" => "Chennai", "del" => "Delhi", "cgk" | "jkt" => "Jakarta",
        "sha" | "pvg" => "Shanghai", "pek" | "bjs" => "Beijing", "ctu" => "Chengdu", "szx" => "Shenzhen", "wuh" => "Wuhan",
        _ => return code.to_uppercase(),
    };
    name.to_string()
}

/// Steam's partner CDNs, by host name.
fn cdn_name(host: &str) -> String {
    let partner = ["akamai", "fastly", "google", "alibaba", "edgenext", "cloudflare", "level3"].into_iter().find(|p| host.contains(p)).unwrap_or("CDN");
    let mut name = partner.to_string();
    if let Some(c) = name.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    format!("{name} (CDN)")
}

/// "cache5-fra2.steamcontent.com" -> "fra".
fn code(host: &str) -> Option<String> {
    let first = host.split('.').next()?;
    let part = first.split('-').nth(1)?;
    let letters: String = part.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    (letters.len() >= 3).then_some(letters.to_lowercase())
}

/// Look up every region and time a connection to one server in each.
pub async fn probe() -> Vec<model::DownloadRegion> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
    // 1) Which regions exist.
    let mut found: BTreeMap<u32, (String, u32)> = BTreeMap::new(); // cell -> (host, load)
    let mut cdns: BTreeMap<String, u32> = BTreeMap::new(); // partner CDN host -> load
    let cells: Vec<u32> = CELLS.collect();
    for chunk in cells.chunks(24) {
        let replies = futures_util::future::join_all(chunk.iter().map(|cell| {
            let http = http.clone();
            async move {
                let v: serde_json::Value = http
                    .get("https://api.steampowered.com/IContentServerDirectoryService/GetServersForSteamPipe/v1/")
                    .query(&[("cell_id", cell.to_string()), ("max_servers", "3".into())])
                    .send()
                    .await
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                Some(v)
            }
        }))
        .await;
        for v in replies.into_iter().flatten() {
            for s in v.pointer("/response/servers").and_then(|s| s.as_array()).into_iter().flatten() {
                let cell = s.get("cell_id").and_then(|c| c.as_u64()).unwrap_or(0) as u32;
                let host = s.get("host").and_then(|h| h.as_str()).unwrap_or_default().to_string();
                let load = s.get("load").and_then(|l| l.as_u64()).unwrap_or(0) as u32;
                match s.get("type").and_then(|t| t.as_str()) {
                    Some("SteamCache") if cell != 0 && code(&host).is_some() => {
                        found.entry(cell).or_insert((host, load));
                    }
                    Some("CDN") if !host.is_empty() => {
                        cdns.entry(host).or_insert(load);
                    }
                    _ => {}
                }
            }
        }
    }
    // 2) Time a TCP connection to each (best of three).
    let mut targets: Vec<(u32, String, u32, Option<String>)> = found.into_iter().map(|(cell, (host, load))| (cell, host, load, None)).collect();
    targets.extend(cdns.into_iter().map(|(host, load)| (0, host.clone(), load, Some(cdn_name(&host)))));
    let timed = futures_util::future::join_all(targets.into_iter().map(|(cell, host, load, cdn)| async move {
        let mut best: Option<u32> = None;
        for _ in 0..3 {
            let start = Instant::now();
            let ok = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect((host.as_str(), 443))).await;
            if matches!(ok, Ok(Ok(_))) {
                let ms = start.elapsed().as_millis() as u32;
                best = Some(best.map_or(ms, |b| b.min(ms)));
            }
        }
        let code = code(&host).unwrap_or_default();
        let name = cdn.unwrap_or_else(|| city(&code));
        model::DownloadRegion { cell_id: cell, name, code, host, load, ping_ms: best }
    }))
    .await;
    let mut out: Vec<model::DownloadRegion> = timed;
    out.sort_by_key(|r| (r.ping_ms.unwrap_or(u32::MAX), r.load));
    // One entry per city: keep the fastest.
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| seen.insert(r.name.clone()));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_server_codes() {
        assert_eq!(super::code("cache5-fra2.steamcontent.com").as_deref(), Some("fra"));
        assert_eq!(super::city("ams"), "Amsterdam");
        assert_eq!(super::code("steampipe.akamaized.net"), None);
    }
}

//! Watching a Steam broadcast: what Steam says about it, and its chat.
//!
//! `/broadcast/getbroadcastmpd/` answers with the state of someone's
//! broadcast and, when it's on, links to the stream. It offers the same video
//! as DASH and as HLS; the app's player already speaks HLS, so that's the one
//! used here. The chat is plain JSON on Steam's CDN, read by polling.
//!
//! Only reads, plus the viewer heartbeat Steam's own player sends so a
//! broadcast keeps counting you. Nothing here posts in the chat.

use crate::market::Web;
use model::{AppId, SteamId};
use rand::Rng;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const COMMUNITY: &str = "https://steamcommunity.com";
/// Chat lines kept in memory.
pub const MAX_CHAT: usize = 150;
/// The longest chat message kept, in characters.
const MAX_MESSAGE: usize = 600;
/// How long to wait for a broadcaster's client to answer (as Steam's page).
const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// A broadcast that's on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub broadcast_id: String,
    pub viewer_token: String,
    /// The HLS master playlist.
    pub url: String,
    pub heartbeat_secs: u64,
}

/// Why there's no picture yet (Steam is still working on it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// Asking the broadcaster's Steam client.
    Answer,
    /// The broadcast hasn't started.
    Start,
    /// The broadcaster lost their connection to Steam.
    Reconnect,
}

/// Why there's nothing to watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Ended,
    /// Steam doesn't say why: not broadcasting, private, or friends only.
    Unavailable,
    NoServers,
    NotSupported,
    Restricted,
    OutOfDate,
    PoorUpload,
    NeedsSubscription,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest {
    Ready(Stream),
    Wait { why: Wait, retry_ms: u64 },
    Stopped(Stop),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Connecting,
    Waiting(Wait),
    Live(Stream),
    Stopped(Stop),
}

/// Title, game and audience, from `/broadcast/getbroadcastinfo/`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    pub title: String,
    pub game: String,
    pub app_id: AppId,
    pub viewers: u32,
    pub thumbnail: String,
    /// Seconds until Steam wants to be asked again.
    pub update_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub steam_id: SteamId,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatState {
    #[default]
    Connecting,
    On,
    /// This broadcast's chat can't be read.
    Unavailable,
}

/// Everything the page shows.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub phase: Phase,
    /// The broadcaster's name, from their broadcast page.
    pub name: String,
    pub info: Info,
    pub chat: VecDeque<ChatLine>,
    pub chat_state: ChatState,
}

/// One broadcast being watched. Clones share it; call [`Watch::stop`] when
/// the page closes, and everything it runs ends.
#[derive(Clone)]
pub struct Watch(Arc<Inner>);

struct Inner {
    steam_id: SteamId,
    started: AtomicBool,
    stopped: AtomicBool,
    view: Mutex<(u64, View)>,
    /// Bumped to wake the task (ask Steam again, or stop).
    wake: watch::Sender<u64>,
    repaint: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Watch").field(&self.0.steam_id).finish()
    }
}

impl Watch {
    pub fn new(steam_id: SteamId, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Watch(Arc::new(Inner {
            steam_id,
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            view: Mutex::new((1, View::default())),
            wake: watch::channel(0).0,
            repaint: Box::new(repaint),
        }))
    }

    pub fn steam_id(&self) -> SteamId {
        self.0.steam_id
    }

    /// A task is running for this watch.
    pub fn started(&self) -> bool {
        self.0.started.load(Ordering::SeqCst)
    }

    /// The latest view, if it changed since `seen`.
    pub fn view_if_changed(&self, seen: &mut u64) -> Option<View> {
        let g = self.0.view.lock().unwrap_or_else(|e| e.into_inner());
        (g.0 != *seen).then(|| {
            *seen = g.0;
            g.1.clone()
        })
    }

    /// Ask Steam about the broadcast again (after the video stopped loading,
    /// or from a "Try again" button).
    pub fn recheck(&self) {
        self.update(|v| v.phase = Phase::Connecting);
        self.0.wake.send_modify(|n| *n += 1);
    }

    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        self.0.wake.send_modify(|n| *n += 1);
    }

    fn stopped(&self) -> bool {
        self.0.stopped.load(Ordering::SeqCst)
    }

    fn update(&self, f: impl FnOnce(&mut View)) {
        {
            let mut g = self.0.view.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut g.1);
            g.0 += 1;
        }
        (self.0.repaint)();
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn number_of(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

fn https(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

/// Read Steam's answer to `getbroadcastmpd`.
pub fn parse_manifest(body: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "Steam sent an unreadable answer".to_string())?;
    let retry_ms = v["retry"].as_u64().unwrap_or(5_000).clamp(1_000, 60_000);
    let wait = |why| Manifest::Wait { why, retry_ms };
    Ok(match v["success"].as_str().unwrap_or_default() {
        "ready" => {
            let url = v["hls_url"].as_str().unwrap_or_default();
            if !https(url) {
                return Ok(Manifest::Stopped(Stop::Failed));
            }
            Manifest::Ready(Stream {
                broadcast_id: text_of(&v["broadcastid"]),
                viewer_token: text_of(&v["viewertoken"]),
                url: url.to_string(),
                heartbeat_secs: v["heartbeat_interval"].as_u64().unwrap_or(30).clamp(10, 300),
            })
        }
        "waiting" => wait(Wait::Answer),
        "waiting_for_start" => wait(Wait::Start),
        "waiting_for_reconnect" => wait(Wait::Reconnect),
        "end" => Manifest::Stopped(Stop::Ended),
        "noservers" => Manifest::Stopped(Stop::NoServers),
        "system_not_supported" => Manifest::Stopped(Stop::NotSupported),
        "user_restricted" => Manifest::Stopped(Stop::Restricted),
        "client_out_of_date" => Manifest::Stopped(Stop::OutOfDate),
        "poor_upload_quality" => Manifest::Stopped(Stop::PoorUpload),
        "missing_subscription" => Manifest::Stopped(Stop::NeedsSubscription),
        "request_failed" => Manifest::Stopped(Stop::Failed),
        // "unavailable", and anything new: what Steam's page calls "not
        // available at this time".
        _ => Manifest::Stopped(Stop::Unavailable),
    })
}

/// Read `getbroadcastinfo`. `Err(())`: Steam says to stop asking (the
/// broadcast is gone).
pub fn parse_info(body: &str) -> Result<Option<Info>, ()> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Ok(None) };
    match v["success"].as_u64() {
        Some(42) => Err(()),
        Some(1) => Ok(Some(Info {
            title: v["title"].as_str().unwrap_or_default().trim().to_string(),
            game: v["app_title"].as_str().unwrap_or_default().trim().to_string(),
            app_id: number_of(&v["appid"]).and_then(|n| AppId::try_from(n).ok()).unwrap_or(0),
            viewers: number_of(&v["viewer_count"]).and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
            thumbnail: v["thumbnail_url"].as_str().filter(|u| https(u)).unwrap_or_default().to_string(),
            update_secs: v["update_interval"].as_u64().unwrap_or(60).clamp(15, 300),
        })),
        _ => Ok(None),
    }
}

/// The chat's link template from `getchatinfo`.
pub fn parse_chat_info(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let url = v["view_url_template"].as_str()?;
    https(&url.replace("{0}", "0")).then(|| url.to_string())
}

/// The link for the chat messages after `next`.
pub fn chat_url(template: &str, next: u64) -> String {
    if template.contains("{0}") {
        template.replace("{0}", &next.to_string())
    } else if next == 0 {
        template.to_string()
    } else {
        format!("{template}{}t={next}", if template.contains('?') { '&' } else { '?' })
    }
}

/// A page of chat.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatPage {
    pub lines: Vec<ChatLine>,
    /// Ask for this position next.
    pub next: u64,
    /// Wait this long first (only on the first page).
    pub delay_ms: Option<u64>,
}

pub fn parse_chat(body: &str) -> Option<ChatPage> {
    let v: Value = serde_json::from_str(body).ok()?;
    let next = number_of(&v["next_request"])?;
    let lines = v["messages"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    let text: String = m["msg"].as_str()?.trim().chars().take(MAX_MESSAGE).collect();
                    (!text.is_empty()).then(|| ChatLine {
                        steam_id: number_of(&m["steamid"]).unwrap_or(0),
                        name: m["persona_name"].as_str().unwrap_or_default().trim().to_string(),
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ChatPage { lines, next, delay_ms: v["initial_delay"].as_u64() })
}

/// The name in a broadcast page's title:
/// `Steam Community :: <name> :: Broadcast`.
pub fn parse_page_name(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let title = &html[start..start + html[start..].find("</title>")?];
    // The last part is the page's name, in the reader's language.
    let (name, _) = title.strip_prefix("Steam Community :: ")?.rsplit_once(" :: ")?;
    let name = crate::hub::strip_tags(name);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Sleep, unless woken. True if woken.
async fn sleep_or_wake(d: Duration, wake: &mut watch::Receiver<u64>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => false,
        r = wake.changed() => r.is_ok(),
    }
}

async fn manifest(web: &Web, id: SteamId, stream: Option<&Stream>) -> Result<Manifest, String> {
    let steam_id = id.to_string();
    let (broadcast, token) = stream.map_or(("0", "0"), |s| (s.broadcast_id.as_str(), s.viewer_token.as_str()));
    let params = [("steamid", steam_id.as_str()), ("broadcastid", broadcast), ("viewertoken", token), ("watchlocation", "5"), ("sessionid", web.session_id.as_str())];
    let url = reqwest::Url::parse_with_params(&format!("{COMMUNITY}/broadcast/getbroadcastmpd/"), params).map_err(|e| e.to_string())?;
    let r = web.http.get(url).timeout(Duration::from_secs(15)).send().await.map_err(|e| e.to_string())?;
    parse_manifest(&r.text().await.map_err(|e| e.to_string())?)
}

/// Ask until Steam gives a stream or a final answer. None if stopped.
async fn connect(web: &Web, w: &Watch, last: Option<&Stream>, wake: &mut watch::Receiver<u64>) -> Option<Phase> {
    let started = Instant::now();
    let mut errors = 0;
    loop {
        if w.stopped() {
            return None;
        }
        let delay = match manifest(web, w.steam_id(), last).await {
            Ok(Manifest::Ready(s)) => return Some(Phase::Live(s)),
            Ok(Manifest::Stopped(s)) => return Some(Phase::Stopped(s)),
            Ok(Manifest::Wait { why, retry_ms }) => {
                errors = 0;
                // Steam's page gives a broadcaster's client a minute to answer.
                if why == Wait::Answer && started.elapsed() > ANSWER_WAIT {
                    return Some(Phase::Stopped(Stop::Unavailable));
                }
                w.update(|v| v.phase = Phase::Waiting(why));
                retry_ms
            }
            Err(e) => {
                tracing::info!("broadcast: {e}");
                errors += 1;
                if errors >= 3 {
                    return Some(Phase::Stopped(Stop::Failed));
                }
                3_000
            }
        };
        sleep_or_wake(Duration::from_millis(delay), wake).await;
    }
}

async fn fetch_name(web: &Web, id: SteamId) -> Option<String> {
    let r = web.http.get(format!("{COMMUNITY}/broadcast/watch/{id}")).timeout(Duration::from_secs(15)).send().await.ok()?;
    parse_page_name(&r.text().await.ok()?)
}

/// Title, game and viewers, as often as Steam asks.
async fn info_task(web: Arc<Web>, w: Watch, broadcast: String) {
    let steam_id = w.steam_id().to_string();
    let Ok(url) = reqwest::Url::parse_with_params(&format!("{COMMUNITY}/broadcast/getbroadcastinfo/"), [("steamid", steam_id.as_str()), ("broadcastid", broadcast.as_str())]) else { return };
    let mut wait = 60;
    loop {
        if let Ok(r) = web.http.get(url.clone()).timeout(Duration::from_secs(15)).send().await {
            match parse_info(&r.text().await.unwrap_or_default()) {
                Ok(Some(info)) => {
                    wait = info.update_secs;
                    w.update(|v| v.info = info);
                }
                Ok(None) => {}
                // It's over: find out how.
                Err(()) => return w.recheck(),
            }
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// Tell Steam we're still watching, as its own player does. A broadcast
/// nobody is counted as watching can stop.
async fn heartbeat_task(web: Arc<Web>, w: Watch, s: Stream) {
    // Spread out, so a broadcast coming back isn't hit by everyone at once.
    let jitter = rand::thread_rng().gen_range(0..10_000);
    tokio::time::sleep(Duration::from_millis(jitter)).await;
    let form = [("steamid", w.steam_id().to_string()), ("broadcastid", s.broadcast_id.clone()), ("viewertoken", s.viewer_token.clone())];
    loop {
        if let Err(e) = web.http.post(format!("{COMMUNITY}/broadcast/heartbeat/")).form(&form).timeout(Duration::from_secs(15)).send().await {
            tracing::debug!("broadcast heartbeat: {e}");
        }
        tokio::time::sleep(Duration::from_secs(s.heartbeat_secs)).await;
    }
}

/// Read the chat until the page closes.
async fn chat_task(web: Arc<Web>, w: Watch, broadcast: String) {
    let steam_id = w.steam_id().to_string();
    let params = [("steamid", steam_id.as_str()), ("broadcastid", broadcast.as_str()), ("sessionid", web.session_id.as_str())];
    let template = match reqwest::Url::parse_with_params(&format!("{COMMUNITY}/broadcast/getchatinfo/"), params) {
        Ok(url) => match web.http.get(url).timeout(Duration::from_secs(15)).send().await {
            Ok(r) => r.text().await.ok().and_then(|b| parse_chat_info(&b)),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let Some(template) = template else {
        return w.update(|v| v.chat_state = ChatState::Unavailable);
    };
    follow_chat(&w, &template).await;
}

/// Add new chat lines, keeping only the latest.
fn add_chat(v: &mut View, lines: Vec<ChatLine>) {
    v.chat_state = ChatState::On;
    v.chat.extend(lines);
    while v.chat.len() > MAX_CHAT {
        v.chat.pop_front();
    }
}

/// Poll the chat's pages, each one saying where the next starts.
async fn follow_chat(w: &Watch, template: &str) {
    // The chat is on a CDN: no reason to send it a sign-in.
    let cdn = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
    let (mut next, mut errors) = (0u64, 0u32);
    loop {
        let page = match cdn.get(chat_url(template, next)).send().await {
            Ok(r) if r.status().is_success() => r.text().await.ok().and_then(|b| parse_chat(&b)),
            _ => None,
        };
        let Some(page) = page else {
            errors += 1;
            if errors >= 5 {
                // Lost our place: start again from now.
                errors = 0;
                next = 0;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        };
        errors = 0;
        let wait = page.delay_ms.unwrap_or_else(|| page.next.saturating_sub(next)).clamp(500, 5_000);
        next = page.next.max(next);
        let lines = page.lines;
        w.update(|v| add_chat(v, lines));
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
}

/// Follow one broadcast until [`Watch::stop`]. `web` is the signed-in session
/// (friends-only broadcasts need it) or an anonymous one for public ones.
pub async fn run(web: Arc<Web>, w: Watch) {
    if w.0.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut wake = w.0.wake.subscribe();
    // For an account Steam can't show, the page's title is just the number.
    if let Some(name) = fetch_name(&web, w.steam_id()).await.filter(|n| *n != w.steam_id().to_string()) {
        w.update(|v| v.name = name);
    }
    let mut last: Option<Stream> = None;
    while !w.stopped() {
        wake.borrow_and_update();
        let Some(phase) = connect(&web, &w, last.as_ref(), &mut wake).await else { return };
        let mut tasks = tokio::task::JoinSet::new();
        if let Phase::Live(s) = &phase {
            // A new broadcast has a new chat; the same one keeps its lines.
            if last.as_ref().is_none_or(|l| l.broadcast_id != s.broadcast_id) {
                w.update(|v| {
                    v.chat.clear();
                    v.chat_state = ChatState::Connecting;
                });
            }
            tasks.spawn(info_task(web.clone(), w.clone(), s.broadcast_id.clone()));
            tasks.spawn(heartbeat_task(web.clone(), w.clone(), s.clone()));
            tasks.spawn(chat_task(web.clone(), w.clone(), s.broadcast_id.clone()));
            last = Some(s.clone());
        }
        w.update(|v| v.phase = phase);
        // Until the page asks again, or closes (which drops the tasks).
        if wake.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like Steam's real answers (ids and hosts changed).
    const READY: &str = r#"{"success":"ready","retry":0,"broadcastid":"1152279812953799800","eresult":1,"url":"https:\/\/cache.example\/broadcast\/1\/2\/manifest\/0\/h\/?o=1","viewertoken":"8792490919011984162","hls_url":"https:\/\/cache.example\/broadcast\/1\/2\/hls_manifest\/0\/h\/master.m3u8?o=1","heartbeat_interval":30,"is_rtmp":1,"title":"","num_viewers":850,"is_webrtc":null,"is_replay":0,"cdn_auth_url_parameters":null}"#;

    #[test]
    fn reads_a_live_broadcast() {
        let Manifest::Ready(s) = parse_manifest(READY).unwrap() else { panic!("not ready") };
        assert_eq!(s.broadcast_id, "1152279812953799800");
        assert_eq!(s.viewer_token, "8792490919011984162");
        assert_eq!(s.url, "https://cache.example/broadcast/1/2/hls_manifest/0/h/master.m3u8?o=1");
        assert_eq!(s.heartbeat_secs, 30);
    }

    #[test]
    fn every_state_steam_names_has_a_meaning() {
        let state = |s: &str| parse_manifest(&format!(r#"{{"success":"{s}","retry":30000,"broadcastid":0,"eresult":1}}"#)).unwrap();
        assert_eq!(state("waiting"), Manifest::Wait { why: Wait::Answer, retry_ms: 30_000 });
        assert_eq!(state("waiting_for_start"), Manifest::Wait { why: Wait::Start, retry_ms: 30_000 });
        assert_eq!(state("waiting_for_reconnect"), Manifest::Wait { why: Wait::Reconnect, retry_ms: 30_000 });
        assert_eq!(state("end"), Manifest::Stopped(Stop::Ended));
        assert_eq!(state("unavailable"), Manifest::Stopped(Stop::Unavailable));
        assert_eq!(state("noservers"), Manifest::Stopped(Stop::NoServers));
        assert_eq!(state("system_not_supported"), Manifest::Stopped(Stop::NotSupported));
        assert_eq!(state("user_restricted"), Manifest::Stopped(Stop::Restricted));
        assert_eq!(state("client_out_of_date"), Manifest::Stopped(Stop::OutOfDate));
        assert_eq!(state("poor_upload_quality"), Manifest::Stopped(Stop::PoorUpload));
        assert_eq!(state("missing_subscription"), Manifest::Stopped(Stop::NeedsSubscription));
        assert_eq!(state("request_failed"), Manifest::Stopped(Stop::Failed));
        assert_eq!(state("something_new"), Manifest::Stopped(Stop::Unavailable));
        assert!(parse_manifest("<html>").is_err());
    }

    #[test]
    fn a_ready_answer_needs_a_safe_stream_link() {
        for bad in ["", "http://cache.example/master.m3u8", "file:///c:/x", "not a url"] {
            let body = format!(r#"{{"success":"ready","hls_url":"{bad}"}}"#);
            assert_eq!(parse_manifest(&body).unwrap(), Manifest::Stopped(Stop::Failed), "{bad}");
        }
    }

    #[test]
    fn retry_times_stay_sane() {
        let wait = |r: &str| parse_manifest(&format!(r#"{{"success":"waiting","retry":{r}}}"#)).unwrap();
        assert_eq!(wait("0"), Manifest::Wait { why: Wait::Answer, retry_ms: 1_000 });
        assert_eq!(wait("999999999"), Manifest::Wait { why: Wait::Answer, retry_ms: 60_000 });
    }

    #[test]
    fn reads_title_game_and_viewers() {
        let body = r#"{"success":1,"appid":"5066010","app_title":"Sample Game","title":" Road to 100 ","viewer_count":881,"permission":3,"is_rtmp":1,"seconds_delay":6,"thumbnail_url":"https:\/\/cdn.example\/thumb\/","update_interval":60,"is_online":true,"is_replay":0}"#;
        let i = parse_info(body).unwrap().unwrap();
        assert_eq!((i.title.as_str(), i.game.as_str(), i.app_id, i.viewers), ("Road to 100", "Sample Game", 5066010, 881));
        assert_eq!((i.thumbnail.as_str(), i.update_secs), ("https://cdn.example/thumb/", 60));
        assert_eq!(parse_info(r#"{"success":42}"#), Err(()));
        assert_eq!(parse_info(r#"{"success":2}"#), Ok(None));
        assert_eq!(parse_info("nope"), Ok(None));
    }

    #[test]
    fn a_thumbnail_must_be_https() {
        let i = parse_info(r#"{"success":1,"thumbnail_url":"http://cdn.example/t"}"#).unwrap().unwrap();
        assert_eq!(i.thumbnail, "");
    }

    #[test]
    fn reads_the_chat() {
        let info = r#"{"success":1,"chat_id":"147","view_url_template":"https:\/\/chat.example\/chat\/147\/messages\/{0}?chat_origin=o:80","blocked":false}"#;
        let template = parse_chat_info(info).unwrap();
        assert_eq!(chat_url(&template, 0), "https://chat.example/chat/147/messages/0?chat_origin=o:80");
        assert_eq!(chat_url(&template, 12631656), "https://chat.example/chat/147/messages/12631656?chat_origin=o:80");
        assert_eq!(chat_url("https://chat.example/m?a=1", 5), "https://chat.example/m?a=1&t=5");
        assert_eq!(chat_url("https://chat.example/m", 5), "https://chat.example/m?t=5");
        assert!(parse_chat_info(r#"{"view_url_template":"http://chat.example/{0}"}"#).is_none());

        let page = parse_chat(r#"{"messages":[{"steamid":"76561197960266732","instance_id":1,"persona_name":" Pixel ","flair":"","in_game":false,"msg":" gg "},{"steamid":"76561197960266733","persona_name":"x","msg":"   "}],"next_request":12631656,"initial_delay":318}"#).unwrap();
        assert_eq!(page.lines, vec![ChatLine { steam_id: 76561197960266732, name: "Pixel".into(), text: "gg".into() }]);
        assert_eq!((page.next, page.delay_ms), (12631656, Some(318)));
        assert!(parse_chat(r#"{"messages":[]}"#).is_none());
    }

    #[test]
    fn a_very_long_chat_message_is_cut() {
        let long = "é".repeat(MAX_MESSAGE * 2);
        let page = parse_chat(&format!(r#"{{"messages":[{{"steamid":"1","persona_name":"x","msg":"{long}"}}],"next_request":1}}"#)).unwrap();
        assert_eq!(page.lines[0].text.chars().count(), MAX_MESSAGE);
    }

    #[test]
    fn finds_the_broadcasters_name() {
        assert_eq!(parse_page_name("<head><title>Steam Community :: Pixel :: Broadcast</title>").as_deref(), Some("Pixel"));
        assert_eq!(parse_page_name("<title>Steam Community :: a :: b :: Broadcast</title>").as_deref(), Some("a :: b"));
        assert_eq!(parse_page_name("<title>Steam Community :: Error</title>"), None);
        assert_eq!(parse_page_name("<title>Sign In</title>"), None);
        assert_eq!(parse_page_name("nothing"), None);
    }

    #[test]
    fn the_chat_keeps_only_the_latest_lines() {
        let mut v = View::default();
        let line = |i: usize| ChatLine { steam_id: 1, name: "x".into(), text: i.to_string() };
        add_chat(&mut v, (0..MAX_CHAT + 5).map(line).collect());
        assert_eq!(v.chat.len(), MAX_CHAT);
        assert_eq!(v.chat.front().unwrap().text, "5");
        assert_eq!(v.chat_state, ChatState::On);
    }

    /// New messages arrive on later pages, each asked for by the position the
    /// page before gave.
    #[tokio::test]
    async fn following_the_chat_adds_new_messages_in_order() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let body = match path.as_str() {
                    "/m/0" => r#"{"messages":[{"steamid":"1","persona_name":"a","msg":"first"}],"next_request":1000,"initial_delay":1}"#,
                    "/m/1000" => r#"{"messages":[{"steamid":"2","persona_name":"b","msg":"second"}],"next_request":1500}"#,
                    _ => r#"{"messages":[],"next_request":2000}"#,
                };
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        let w = Watch::new(1, || {});
        let follower = w.clone();
        let task = tokio::spawn(async move { follow_chat(&follower, &format!("http://{addr}/m/{{0}}")).await });
        let mut seen = 0;
        let mut texts = Vec::new();
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(v) = w.view_if_changed(&mut seen) {
                texts = v.chat.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
            }
            if texts.len() >= 2 && asked.lock().unwrap().len() >= 3 {
                break;
            }
        }
        task.abort();
        server.abort();
        assert_eq!(texts, ["first", "second"]);
        assert_eq!(asked.lock().unwrap()[..3], ["/m/0", "/m/1000", "/m/1500"]);
    }

    #[test]
    fn a_watch_reports_changes_once() {
        let w = Watch::new(1, || {});
        let mut seen = 0;
        assert!(w.view_if_changed(&mut seen).is_some());
        assert!(w.view_if_changed(&mut seen).is_none());
        w.update(|v| v.name = "Pixel".into());
        assert_eq!(w.view_if_changed(&mut seen).unwrap().name, "Pixel");
    }
}

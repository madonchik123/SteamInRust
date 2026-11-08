//! A live CM connection. Cheap to clone; all clones share one socket.

use crate::{Error, Packet, Result, emsg};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use proto::{CMsgProtoBufHeader, NO_JOB};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: String,
    out: mpsc::UnboundedSender<Vec<u8>>,
    jobs: Mutex<HashMap<u64, mpsc::UnboundedSender<Packet>>>,
    next_job: AtomicU64,
    steam_id: AtomicU64,
    session_id: AtomicI32,
}

/// Incoming messages that are not replies to a request (friend updates, etc).
pub type Events = mpsc::UnboundedReceiver<Packet>;

impl Connection {
    /// Open a WebSocket to one CM endpoint (`host:port`).
    pub async fn connect(endpoint: &str) -> Result<(Connection, Events)> {
        let url = format!("wss://{endpoint}/cmsocket/");
        tracing::info!("connecting to {url}");
        let (ws, _) = match proxy() {
            // Through an HTTP proxy (STEAMINRUST_PROXY).
            Some(proxy) => {
                let tcp = tunnel(&proxy, endpoint).await?;
                tokio_tungstenite::client_async_tls(url.as_str(), tcp).await?
            }
            None => tokio_tungstenite::connect_async(url.as_str()).await?,
        };
        let (mut sink, mut stream) = ws.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let inner = Arc::new(Inner {
            endpoint: endpoint.to_string(),
            out: out_tx,
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            steam_id: AtomicU64::new(0),
            session_id: AtomicI32::new(0),
        });

        // Writer: forward queued frames to the socket.
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                if let Err(e) = sink.send(WsMessage::Binary(frame.into())).await {
                    tracing::warn!("send failed: {e}");
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // Reader: decode frames, route job replies, forward everything else.
        let reader = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let data = match msg {
                    Ok(WsMessage::Binary(b)) => b,
                    Ok(WsMessage::Close(frame)) => {
                        tracing::info!("server closed connection: {frame:?}");
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        tracing::warn!("socket error: {e}");
                        break;
                    }
                };
                let Some(inner) = reader.upgrade() else { break };
                let packets = match Packet::decode(&data) {
                    Ok(Some(p)) if p.emsg == emsg::MULTI => p.unpack_multi(),
                    Ok(Some(p)) => Ok(vec![p]),
                    Ok(None) => Ok(vec![]),
                    Err(e) => Err(e),
                };
                match packets {
                    Ok(packets) => {
                        for p in packets {
                            inner.route(p, &event_tx);
                        }
                    }
                    Err(e) => tracing::warn!("bad frame: {e}"),
                }
            }
            // Dropping senders wakes every pending request with Disconnected.
            if let Some(inner) = reader.upgrade() {
                inner.jobs.lock().unwrap().clear();
            }
        });

        Ok((Connection { inner }, event_rx))
    }

    /// A connection that is already gone: every send fails at once with
    /// [`Error::Disconnected`]. Stands in for the real one while offline.
    pub fn closed() -> Connection {
        let (out, _) = mpsc::unbounded_channel();
        Connection {
            inner: Arc::new(Inner {
                endpoint: String::new(),
                out,
                jobs: Mutex::new(HashMap::new()),
                next_job: AtomicU64::new(1),
                steam_id: AtomicU64::new(0),
                session_id: AtomicI32::new(0),
            }),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }

    pub fn steam_id(&self) -> u64 {
        self.inner.steam_id.load(Ordering::Relaxed)
    }

    pub fn is_logged_on(&self) -> bool {
        self.inner.session_id.load(Ordering::Relaxed) != 0
    }

    /// Remember who we are after `ClientLogon` succeeds; stamped on every
    /// following message header.
    pub fn set_session(&self, steam_id: u64, session_id: i32) {
        self.inner.steam_id.store(steam_id, Ordering::Relaxed);
        self.inner.session_id.store(session_id, Ordering::Relaxed);
    }

    fn header(&self) -> CMsgProtoBufHeader {
        let sid = self.inner.session_id.load(Ordering::Relaxed);
        CMsgProtoBufHeader {
            steamid: Some(self.steam_id()),
            client_sessionid: (sid != 0).then_some(sid),
            ..Default::default()
        }
    }

    /// Send a message without waiting for a reply.
    pub fn send(&self, emsg: u32, body: &impl Message) -> Result<()> {
        self.send_with(emsg, self.header(), body)
    }

    /// Send with a custom header (fields left at defaults are filled in).
    pub fn send_with(&self, emsg: u32, mut header: CMsgProtoBufHeader, body: &impl Message) -> Result<()> {
        let base = self.header();
        header.steamid = header.steamid.or(base.steamid);
        header.client_sessionid = header.client_sessionid.or(base.client_sessionid);
        tracing::debug!("-> {}", emsg::name(emsg));
        self.inner
            .out
            .send(Packet::encode(emsg, &header, body))
            .map_err(|_| Error::Disconnected)
    }

    fn start_job(&self) -> (u64, mpsc::UnboundedReceiver<Packet>, JobGuard) {
        let id = self.inner.next_job.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner.jobs.lock().unwrap().insert(id, tx);
        (id, rx, JobGuard { inner: self.inner.clone(), id })
    }

    /// Send a message and wait for the reply that targets its job id.
    pub async fn request(&self, emsg: u32, body: &impl Message) -> Result<Packet> {
        self.request_with(emsg, CMsgProtoBufHeader::default(), body).await
    }

    pub async fn request_with(
        &self,
        emsg: u32,
        mut header: CMsgProtoBufHeader,
        body: &impl Message,
    ) -> Result<Packet> {
        let (id, mut rx, _guard) = self.start_job();
        header.jobid_source = Some(id);
        self.send_with(emsg, header, body)?;
        match tokio::time::timeout(DEFAULT_TIMEOUT, rx.recv()).await {
            Ok(Some(p)) => Ok(p),
            Ok(None) => Err(Error::Disconnected),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Like [`request`](Self::request), but collects replies until `done` says
    /// the last one arrived (PICS sends large answers in several parts).
    pub async fn request_many(
        &self,
        emsg: u32,
        body: &impl Message,
        done: impl Fn(&Packet) -> bool,
    ) -> Result<Vec<Packet>> {
        let (id, mut rx, _guard) = self.start_job();
        let header = CMsgProtoBufHeader { jobid_source: Some(id), ..Default::default() };
        self.send_with(emsg, header, body)?;
        let mut out = Vec::new();
        loop {
            match tokio::time::timeout(DEFAULT_TIMEOUT, rx.recv()).await {
                Ok(Some(p)) => {
                    let finished = done(&p);
                    out.push(p);
                    if finished {
                        return Ok(out);
                    }
                }
                Ok(None) => return Err(Error::Disconnected),
                Err(_) => return Err(Error::Timeout),
            }
        }
    }

    /// Call a service method that has no reply (Steam's "notification"
    /// methods, e.g. `SteamNotification.MarkNotificationsRead#1`).
    pub fn notify_service<Req: Message>(&self, method: &str, req: &Req) -> Result<()> {
        let header = CMsgProtoBufHeader { target_job_name: Some(method.to_string()), ..Default::default() };
        self.send_with(emsg::SERVICE_METHOD_CALL_FROM_CLIENT, header, req)
    }

    /// Like [`call`](Self::call) but returns the raw reply, for methods that
    /// report errors in the body as well as the header.
    pub async fn call_packet<Req: Message>(&self, method: &str, req: &Req) -> Result<Packet> {
        let emsg = if self.is_logged_on() { emsg::SERVICE_METHOD_CALL_FROM_CLIENT } else { emsg::SERVICE_METHOD_CALL_FROM_CLIENT_NON_AUTHED };
        let header = CMsgProtoBufHeader { target_job_name: Some(method.to_string()), ..Default::default() };
        self.request_with(emsg, header, req).await
    }

    /// Call a "unified" service method, e.g. `Authentication.BeginAuthSessionViaCredentials#1`.
    pub async fn call<Req: Message, Resp: Message + Default>(
        &self,
        method: &str,
        req: &Req,
    ) -> Result<Resp> {
        let emsg = if self.is_logged_on() {
            emsg::SERVICE_METHOD_CALL_FROM_CLIENT
        } else {
            emsg::SERVICE_METHOD_CALL_FROM_CLIENT_NON_AUTHED
        };
        let header = CMsgProtoBufHeader {
            target_job_name: Some(method.to_string()),
            ..Default::default()
        };
        let reply = self.request_with(emsg, header, req).await?;
        reply.check()?.body()
    }
}

/// `host:port` of the HTTP proxy for the Steam connection, from
/// `STEAMINRUST_PROXY` (`http://host:port`). Opt-in, so a system-wide
/// `HTTPS_PROXY` doesn't change how the app reaches Steam.
fn proxy() -> Option<String> {
    let raw = std::env::var("STEAMINRUST_PROXY").ok().filter(|v| !v.trim().is_empty())?;
    let rest = raw.trim().strip_prefix("http://").unwrap_or(raw.trim());
    let host = rest.split('/').next().unwrap_or(rest);
    (!host.is_empty() && !rest.starts_with("socks")).then(|| host.to_string())
}

/// A TCP tunnel to `endpoint` through an HTTP proxy (`CONNECT`).
async fn tunnel(proxy: &str, endpoint: &str) -> Result<tokio::net::TcpStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = tokio::net::TcpStream::connect(proxy).await?;
    tcp.write_all(format!("CONNECT {endpoint} HTTP/1.1\r\nHost: {endpoint}\r\n\r\n").as_bytes()).await?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if tcp.read(&mut byte).await? == 0 || head.len() > 8192 {
            return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "proxy closed the connection")));
        }
        head.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&head);
    if !status.lines().next().is_some_and(|l| l.split_whitespace().nth(1) == Some("200")) {
        return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, format!("proxy refused: {}", status.lines().next().unwrap_or("")))));
    }
    Ok(tcp)
}

impl Inner {
    fn route(&self, p: Packet, events: &mpsc::UnboundedSender<Packet>) {
        tracing::debug!("<- {}", emsg::name(p.emsg));
        let target = p.header.jobid_target.unwrap_or(NO_JOB);
        if target != NO_JOB {
            if let Some(tx) = self.jobs.lock().unwrap().get(&target) {
                let _ = tx.send(p);
                return;
            }
        }
        let _ = events.send(p);
    }
}

/// Removes a job from the routing table when the request finishes or is dropped.
struct JobGuard {
    inner: Arc<Inner>,
    id: u64,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.inner.jobs.lock().unwrap().remove(&self.id);
    }
}

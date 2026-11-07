//! Content servers (Steam's CDN). Servers are ranked by measured speed, and a
//! failing server is skipped automatically.

use crate::{Error, Result};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Server {
    pub host: String,
    pub https: bool,
    /// Steam's reported load (lower is better).
    pub load: f32,
}

pub struct Cdn {
    http: reqwest::Client,
    /// Servers best-first; reordered as we learn which are fast.
    servers: Mutex<Vec<(Server, f64)>>,
}

impl Cdn {
    pub fn new(servers: Vec<Server>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("Valve/Steam HTTP Client 1.0")
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(32)
            .build()
            .expect("http client");
        let mut s: Vec<(Server, f64)> = servers.into_iter().map(|s| (s, 0.0)).collect();
        s.sort_by(|a, b| a.0.load.total_cmp(&b.0.load));
        Self { http, servers: Mutex::new(s) }
    }

    pub fn server_count(&self) -> usize {
        self.servers.lock().unwrap().len()
    }

    /// Measure each server with a tiny request and put the fastest first.
    /// (Steam just takes the directory's order; this is noticeably quicker.)
    pub async fn rank(&self) {
        let list: Vec<Server> = self.servers.lock().unwrap().iter().map(|s| s.0.clone()).collect();
        let mut timed = Vec::new();
        for s in list.into_iter().take(8) {
            let url = format!("{}://{}/", if s.https { "https" } else { "http" }, s.host);
            let start = Instant::now();
            let ok = tokio::time::timeout(Duration::from_secs(3), self.http.head(&url).send()).await.is_ok();
            let ms = if ok { start.elapsed().as_secs_f64() * 1000.0 } else { 10_000.0 };
            timed.push((s, ms));
        }
        if !timed.is_empty() {
            timed.sort_by(|a, b| a.1.total_cmp(&b.1));
            tracing::info!("fastest content server: {} ({:.0} ms)", timed[0].0.host, timed[0].1);
            let mut servers = self.servers.lock().unwrap();
            let rest: Vec<(Server, f64)> = servers.drain(..).filter(|s| !timed.iter().any(|t| t.0.host == s.0.host)).collect();
            servers.extend(timed);
            servers.extend(rest);
        }
    }

    fn pick(&self, attempt: usize) -> Result<Server> {
        let servers = self.servers.lock().unwrap();
        if servers.is_empty() {
            return Err(Error::NoServers);
        }
        Ok(servers[attempt % servers.len().min(4)].0.clone())
    }

    fn demote(&self, host: &str) {
        let mut servers = self.servers.lock().unwrap();
        if let Some(i) = servers.iter().position(|s| s.0.host == host) {
            let s = servers.remove(i);
            servers.push(s);
        }
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let mut last = None;
        for attempt in 0..5 {
            let s = self.pick(attempt)?;
            let url = format!("{}://{}{}", if s.https { "https" } else { "http" }, s.host, path);
            match self.http.get(&url).send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => match r.bytes().await {
                    Ok(b) => return Ok(b.to_vec()),
                    Err(e) => last = Some(e),
                },
                Err(e) => {
                    if e.status().map(|s| s.as_u16()) == Some(401) || e.status().map(|s| s.as_u16()) == Some(403) {
                        return Err(Error::Http(e));
                    }
                    last = Some(e);
                }
            }
            self.demote(&s.host);
            tokio::time::sleep(Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        Err(last.map(Error::Http).unwrap_or(Error::NoServers))
    }

    pub async fn manifest(&self, depot: u32, gid: u64, request_code: u64) -> Result<Vec<u8>> {
        let path = if request_code > 0 {
            format!("/depot/{depot}/manifest/{gid}/5/{request_code}")
        } else {
            format!("/depot/{depot}/manifest/{gid}/5")
        };
        self.get(&path).await
    }

    pub async fn chunk(&self, depot: u32, sha: &[u8]) -> Result<Vec<u8>> {
        self.get(&format!("/depot/{depot}/chunk/{}", crate::chunk::hex(sha))).await
    }
}

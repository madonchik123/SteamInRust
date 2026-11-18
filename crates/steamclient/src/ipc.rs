//! Client side of the pipe to SteamInRust: one connection per request.

use crate::protocol::{ENV_IPC, Envelope, Reply, Request};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

const MAX_IPC_REPLY: u64 = 64 << 20;

/// `(pipe path, token)` from the environment SteamInRust gave the game.
pub fn target() -> Option<(String, String)> {
    let v = std::env::var(ENV_IPC).ok()?;
    let (pipe, token) = v.split_once('|')?;
    (!pipe.is_empty() && !token.is_empty()).then(|| (pipe.to_string(), token.to_string()))
}

/// The pipe (Windows) or socket file (macOS, Linux) SteamInRust listens on.
#[cfg(windows)]
fn open(pipe: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(pipe)
}

#[cfg(unix)]
fn open(pipe: &str) -> std::io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(pipe)
}

/// Send one request and wait for the answer. `None` if SteamInRust can't be
/// reached (closed, signed out) or answered nonsense.
pub fn request(req: Request) -> Option<Reply> {
    let (pipe, token) = target()?;
    let what = format!("{req:?}");
    let what = &what[..what.len().min(60)];
    let started = Instant::now();
    // The server makes a new pipe instance after each connection; a busy
    // pipe just means another request is being accepted right now.
    let file = loop {
        match open(&pipe) {
            Ok(f) => break f,
            Err(e) if started.elapsed() < Duration::from_secs(3) && (matches!(e.raw_os_error(), Some(231) | Some(2)) || e.kind() == std::io::ErrorKind::ConnectionRefused) => {
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(e) => {
                crate::log!("ipc: can't open {pipe}: {e}");
                return None;
            }
        }
    };
    exchange(&file, token, req, what, started)
}

/// Write the request line, read the reply line.
fn exchange<S>(conn: S, token: String, req: Request, what: &str, started: Instant) -> Option<Reply>
where
    S: Read + Write + Copy,
{
    let env = Envelope { token, pid: std::process::id(), req };
    let mut line = serde_json::to_string(&env).ok()?;
    line.push('\n');
    let mut w = conn;
    if let Err(e) = w.write_all(line.as_bytes()).and_then(|_| w.flush()) {
        crate::log!("ipc: write failed: {e}");
        return None;
    }
    let mut reply = String::new();
    if let Err(e) = BufReader::new(conn.take(MAX_IPC_REPLY)).read_line(&mut reply) {
        crate::log!("ipc: read failed: {e}");
        return None;
    }
    if !reply.ends_with('\n') {
        crate::log!("ipc: reply truncated or exceeded 64 MiB limit");
        return None;
    }
    let parsed = serde_json::from_str::<Reply>(reply.trim());
    crate::log!("ipc: {what} -> {} bytes in {} ms", reply.len(), started.elapsed().as_millis());
    parsed.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FloodPipe {
        read_bytes: AtomicUsize,
        limit: usize,
    }

    impl Read for &FloodPipe {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let cur = self.read_bytes.load(Ordering::Relaxed);
            if cur >= self.limit {
                return Ok(0);
            }
            let n = buf.len().min(self.limit - cur);
            buf[..n].fill(b'a');
            self.read_bytes.fetch_add(n, Ordering::Relaxed);
            Ok(n)
        }
    }

    impl Write for &FloodPipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn exchange_bounds_reply_at_64_mib() {
        let pipe = FloodPipe { read_bytes: AtomicUsize::new(0), limit: 68 << 20 };
        let got = exchange(&pipe, "tok".into(), Request::Poll, "Poll", Instant::now());
        assert!(got.is_none());
        let read = pipe.read_bytes.load(Ordering::Relaxed);
        assert!(read <= (64 << 20) + 16 * 1024, "read_line was not bounded at 64 MiB: read {read} bytes");
    }
}

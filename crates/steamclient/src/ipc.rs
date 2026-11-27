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

/// A `tcp:<address>` target: the localhost connection SteamInRust offers
/// games that run through Wine on a Mac, where its pipe or socket can't be
/// reached. Only loopback addresses count.
pub fn tcp_target(pipe: &str) -> Option<std::net::SocketAddr> {
    let addr: std::net::SocketAddr = pipe.strip_prefix("tcp:")?.parse().ok()?;
    addr.ip().is_loopback().then_some(addr)
}

/// Send one request and wait for the answer. `None` if SteamInRust can't be
/// reached (closed, signed out) or answered nonsense.
pub fn request(req: Request) -> Option<Reply> {
    let (pipe, token) = target()?;
    let what = format!("{req:?}");
    let what = &what[..what.len().min(60)];
    let started = Instant::now();
    if pipe.starts_with("tcp:") {
        let Some(addr) = tcp_target(&pipe) else {
            crate::log!("ipc: refusing {pipe}: not a localhost address");
            return None;
        };
        let stream = loop {
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => break s,
                Err(e) if started.elapsed() < Duration::from_secs(3) && e.kind() == std::io::ErrorKind::ConnectionRefused => std::thread::sleep(Duration::from_millis(30)),
                Err(e) => {
                    crate::log!("ipc: can't connect to {addr}: {e}");
                    return None;
                }
            }
        };
        let _ = stream.set_nodelay(true);
        return exchange(&stream, token, req, what, started);
    }
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
    fn tcp_targets_must_be_loopback() {
        assert_eq!(tcp_target("tcp:127.0.0.1:5000"), Some("127.0.0.1:5000".parse().unwrap()));
        assert_eq!(tcp_target("tcp:[::1]:5000"), Some("[::1]:5000".parse().unwrap()));
        assert_eq!(tcp_target("tcp:192.168.1.2:5000"), None);
        assert_eq!(tcp_target("tcp:0.0.0.0:5000"), None);
        assert_eq!(tcp_target("tcp:example.com:5000"), None);
        assert_eq!(tcp_target(r"\\.\pipe\x"), None);
    }

    #[test]
    fn talks_over_localhost_tcp() {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&s).read_line(&mut line).unwrap();
            let env: crate::protocol::Envelope = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(env.token, "tok");
            let reply = serde_json::to_string(&Reply::Events { events: vec![] }).unwrap();
            (&s).write_all(format!("{reply}\n").as_bytes()).unwrap();
        });
        let stream = std::net::TcpStream::connect(addr).unwrap();
        let got = exchange(&stream, "tok".into(), Request::Poll, "Poll", Instant::now());
        server.join().unwrap();
        assert!(matches!(got, Some(Reply::Events { .. })), "{got:?}");
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

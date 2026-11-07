//! Client side of the pipe to SteamInRust: one connection per request.

use crate::protocol::{ENV_IPC, Envelope, Reply, Request};
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

/// `(pipe path, token)` from the environment SteamInRust gave the game.
pub fn target() -> Option<(String, String)> {
    let v = std::env::var(ENV_IPC).ok()?;
    let (pipe, token) = v.split_once('|')?;
    (!pipe.is_empty() && !token.is_empty()).then(|| (pipe.to_string(), token.to_string()))
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
        match std::fs::OpenOptions::new().read(true).write(true).open(&pipe) {
            Ok(f) => break f,
            Err(e) if started.elapsed() < Duration::from_secs(3) && matches!(e.raw_os_error(), Some(231) | Some(2)) => {
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(e) => {
                crate::log!("ipc: can't open {pipe}: {e}");
                return None;
            }
        }
    };
    let env = Envelope { token, pid: std::process::id(), req };
    let mut line = serde_json::to_string(&env).ok()?;
    line.push('\n');
    let mut w = &file;
    if let Err(e) = w.write_all(line.as_bytes()).and_then(|_| w.flush()) {
        crate::log!("ipc: write failed: {e}");
        return None;
    }
    let mut reply = String::new();
    if let Err(e) = BufReader::new(&file).read_line(&mut reply) {
        crate::log!("ipc: read failed: {e}");
        return None;
    }
    let parsed = serde_json::from_str::<Reply>(reply.trim());
    crate::log!("ipc: {what} -> {} bytes in {} ms", reply.len(), started.elapsed().as_millis());
    parsed.ok()
}

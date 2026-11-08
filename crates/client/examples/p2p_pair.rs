//! Two players on one PC, no Steam account needed: checks the steamclient
//! DLL's P2P networking (ISteamNetworkingSockets / Messages) end to end.
//!
//! `cargo run -p client --example p2p_pair -- <probe.exe> <steamclient.dll>`
//!
//! Plays SteamInRust for two made-up players: answers each game's Hello and
//! relays their P2P signalling like Steam would. The probes (the steamclient
//! crate's `probe` example in `--direct` mode) load the DLL themselves, so
//! nothing in the registry changes. Data goes over real UDP between them.

use client::ipc::Server;
use client::ipc::protocol::{GameEvent, Reply, Request, Session};
use std::process::{Command, Stdio};

const HOST: u64 = 76561197960265729;
const JOIN: u64 = 76561197960265730;

fn main() {
    let mut args = std::env::args().skip(1);
    let probe = args.next().expect("probe.exe");
    let dll = args.next().expect("steamclient.dll");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let (server, mut calls) = Server::start();
        let host_token = server.register(480);
        let join_token = server.register(480);
        let start = |role: &str, token: &str, peer: u64| {
            Command::new(&probe)
                .args(["--direct", &dll, "--role", role, "--peer", &peer.to_string()])
                .env("STEAMINRUST_IPC", server.env_value(token))
                .stdout(Stdio::inherit())
                .spawn()
                .expect("start probe")
        };
        let mut host = start("host", &host_token, JOIN);
        let mut join = start("join", &join_token, HOST);
        let done = tokio::task::spawn_blocking(move || (host.wait().map(|s| s.success()).unwrap_or(false), join.wait().map(|s| s.success()).unwrap_or(false)));
        tokio::pin!(done);
        loop {
            tokio::select! {
                r = &mut done => {
                    let (h, j) = r.unwrap();
                    println!("host {}, join {}", if h { "passed" } else { "FAILED" }, if j { "passed" } else { "FAILED" });
                    std::process::exit(if h && j { 0 } else { 1 });
                }
                Some(call) = calls.recv() => {
                    let me = if call.token == host_token { HOST } else { JOIN };
                    let other = if call.token == host_token { &join_token } else { &host_token };
                    let reply = match call.req {
                        Request::Hello => Reply::Hello(Box::new(Session {
                            app_id: 480,
                            steam_id: me,
                            account: "test".into(),
                            persona_name: if me == HOST { "Host".into() } else { "Joiner".into() },
                            country: "US".into(),
                            ui_language: "english".into(),
                            game_language: "english".into(),
                            owned: vec![480],
                            ..Default::default()
                        })),
                        // Steam's part: carry signalling to the other player.
                        Request::Signal { data, .. } => {
                            server.push_to(other, GameEvent::Signal { from: me, data });
                            Reply::Ok
                        }
                        _ => Reply::Err { message: "not in this test".into() },
                    };
                    let _ = call.reply.send(reply);
                }
            }
        }
    });
}

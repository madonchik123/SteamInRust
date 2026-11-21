//! Game servers hosted by games on our steamclient (`ISteamGameServer`).
//!
//! A game server is its own Steam identity, separate from the player: an
//! anonymous one, or a persistent one from a game server login token. Each
//! running server gets its own connection to Steam here, signs in, tells
//! Steam its details so the server browser lists it, and checks players'
//! tickets. The server is never reported as VAC-secured: games with VAC
//! never run on our steamclient.

use crate::ipc::protocol::{GameEvent, GsInfo, Reply, Request};
use crate::ipc::{Auth, Pusher, Server, SharedAuth};
use model::AppId;
use net::{Connection, emsg, eresult};
use proto::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const EMSG_LOGON_GAME_SERVER: u32 = 5559;
const EMSG_GS_SERVER_TYPE: u32 = 908;
const EMSG_GS_PLAYER_LIST: u32 = 909;
const EMSG_GAME_SERVER_UPDATE: u32 = 4331;
const EMSG_GAME_SERVER_REMOVE: u32 = 4332;
const EMSG_P2P_CONNECTION_FAIL: u32 = 5435;

/// k_EResultNoConnection.
const NO_CONNECTION: i32 = 3;
const PROTOCOL_VERSION: u32 = 65580;
const OS_TYPE_WIN11: u32 = 20;
/// What Steam's own game servers report as the server-browser revision.
const LISTING_REVISION: u32 = 17;
/// How often the listing is refreshed while nothing changes.
const LISTING_EVERY: Duration = Duration::from_secs(30);

/// Steam ID to sign in with: universe 1, account type 4 (anonymous game
/// server) or 3 (game server with a login token); Steam fills in the rest.
fn logon_steam_id(token: &str) -> u64 {
    let kind: u64 = if token.is_empty() { 4 } else { 3 };
    (1 << 56) | (kind << 52)
}

/// The sign-in message and its id. A token signs in with its own message.
fn logon_message(app_id: AppId, token: &str) -> (u32, CMsgClientLogon) {
    let msg = CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        client_os_type: Some(OS_TYPE_WIN11),
        game_server_app_id: Some(app_id as i32),
        game_server_token: (!token.is_empty()).then(|| token.to_string()),
        ..Default::default()
    };
    (if token.is_empty() { emsg::CLIENT_LOGON } else { EMSG_LOGON_GAME_SERVER }, msg)
}

fn server_type(i: &GsInfo) -> CMsgGsServerType {
    CMsgGsServerType {
        app_id_served: Some(i.app_id),
        flags: Some(server_flags(i)),
        game_port: Some(i.game_port as u32),
        game_query_port: Some(i.query_port as u32),
        game_dir: Some(i.mod_dir.clone()),
        game_version: Some(i.version.clone()),
        ..Default::default()
    }
}

/// k_unServerFlag*: active, dedicated, passworded. Never "secure" (VAC).
fn server_flags(i: &GsInfo) -> u32 {
    (i.active as u32) | ((i.dedicated as u32) << 2) | ((i.password as u32) << 4)
}

/// What the master server stores for the server browser.
fn listing(i: &GsInfo) -> CMsgGameServerData {
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    CMsgGameServerData {
        revision: Some(LISTING_REVISION),
        query_port: Some(i.query_port as u32),
        game_port: Some(i.game_port as u32),
        spectator_port: Some(i.spectator_port as u32),
        server_name: text(&i.name),
        game_description: text(&i.description),
        spectator_server_name: text(&i.spectator_name),
        app_id: Some(i.app_id),
        gamedir: text(&i.mod_dir),
        version: text(&i.version),
        product: text(&i.product),
        region: text(&i.region),
        players: i.players.iter().map(|&id| c_msg_game_server_data::Player { steam_id: Some(id) }).collect(),
        max_players: Some(i.max_players.max(0) as u32),
        bot_count: Some(i.bots.max(0) as u32),
        password: Some(i.password),
        secure: Some(false),
        dedicated: Some(i.dedicated),
        os: Some(if cfg!(windows) { "w" } else if cfg!(target_os = "macos") { "m" } else { "l" }.to_string()),
        game_data: text(&i.game_data),
        game_type: text(&i.tags),
        map: text(&i.map),
        ..Default::default()
    }
}

fn player_list(i: &GsInfo) -> CMsgGsPlayerList {
    CMsgGsPlayerList { players: i.players.iter().map(|&id| c_msg_gs_player_list::Player { steam_id: Some(id), ..Default::default() }).collect() }
}

fn public_ip(r: &CMsgClientLogonResponse) -> u32 {
    match r.public_ip.as_ref().and_then(|ip| ip.ip.as_ref()) {
        Some(c_msg_ip_address::Ip::V4(v)) => *v,
        _ => 0,
    }
}

// --------------------------------------------------- one server's task

enum Cmd {
    Update(Box<GsInfo>),
    BeginAuth { steam_id: u64, ticket: Vec<u8>, reply: oneshot::Sender<Reply> },
    EndAuth { steam_id: u64 },
    UserStats { steam_id: u64, reply: oneshot::Sender<Reply> },
    StoreStats { steam_id: u64, stats: Vec<(u32, u32)>, crc: u32, reply: oneshot::Sender<Reply> },
    Cert { public_key: Vec<u8>, reply: oneshot::Sender<Reply> },
    Rendezvous { to: u64, data: Vec<u8> },
    LogOff,
}

impl Cmd {
    /// Answer a request that can't run because Steam isn't connected.
    fn refuse(self) {
        let gone = || Reply::Err { message: "The game server isn't connected to Steam".into() };
        match self {
            // k_EAuthSessionResponseUserNotConnectedToSteam
            Cmd::BeginAuth { reply, .. } => drop(reply.send(Reply::AuthResult { response: 1, owner: 0 })),
            Cmd::UserStats { reply, .. } => drop(reply.send(Reply::Stats { result: Err("not connected".into()) })),
            Cmd::StoreStats { crc, reply, .. } => drop(reply.send(Reply::Stored { eresult: NO_CONNECTION, crc })),
            Cmd::Cert { reply, .. } => drop(reply.send(gone())),
            _ => {}
        }
    }
}

struct Signed {
    conn: Connection,
    events: net::connection::Events,
    steam_id: u64,
    public_ip: u32,
    heartbeat: u64,
}

/// Connect and sign in; `Err(EResult)` if Steam (or the network) says no.
async fn sign_in(app_id: AppId, token: &str) -> Result<Signed, i32> {
    let (conn, mut events) = crate::session::connect_any().await.map_err(|e| {
        tracing::warn!("game server {app_id}: can't reach Steam: {e}");
        NO_CONNECTION
    })?;
    let _ = conn.send(emsg::CLIENT_HELLO, &CMsgClientHello { protocol_version: Some(PROTOCOL_VERSION) });
    conn.set_session(logon_steam_id(token), 0);
    let (id, msg) = logon_message(app_id, token);
    conn.send(id, &msg).map_err(|_| NO_CONNECTION)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let p = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(p)) => p,
            _ => return Err(NO_CONNECTION),
        };
        if p.emsg != emsg::CLIENT_LOGON_RESPONSE {
            continue;
        }
        let r: CMsgClientLogonResponse = p.body().map_err(|_| eresult::FAIL)?;
        if r.eresult() != eresult::OK {
            tracing::warn!("game server {app_id}: Steam refused the sign-in ({})", r.eresult());
            return Err(r.eresult());
        }
        let steam_id = p.header.steamid();
        conn.set_session(steam_id, p.header.client_sessionid());
        tracing::info!("game server {app_id}: signed in as {steam_id}");
        return Ok(Signed { conn, events, steam_id, public_ip: public_ip(&r), heartbeat: r.heartbeat_seconds().clamp(5, 60) as u64 });
    }
}

enum Ended {
    LoggedOff,
    Lost(i32),
}

/// Keep one signed-in connection going until it drops or the game logs off.
async fn serve(s: Signed, app_id: AppId, pid: u32, info: &mut GsInfo, cmds: &mut mpsc::UnboundedReceiver<Cmd>, push: &Pusher) -> Ended {
    let Signed { conn, mut events, steam_id: me, public_ip, heartbeat } = s;
    let auth: SharedAuth = Auth::new(public_ip);
    let _ = conn.send(EMSG_GS_SERVER_TYPE, &server_type(info));
    let mut listed = info.active;
    if listed {
        let _ = conn.send(EMSG_GAME_SERVER_UPDATE, &listing(info));
    }
    let mut beat = tokio::time::interval(Duration::from_secs(heartbeat));
    let mut refresh = tokio::time::interval(LISTING_EVERY);
    refresh.tick().await;
    loop {
        tokio::select! {
            _ = beat.tick() => {
                if conn.send(emsg::CLIENT_HEARTBEAT, &CMsgClientHeartBeat { send_reply: Some(false) }).is_err() {
                    return Ended::Lost(NO_CONNECTION);
                }
            }
            _ = refresh.tick() => {
                if listed {
                    let _ = conn.send(EMSG_GAME_SERVER_UPDATE, &listing(info));
                }
            }
            p = events.recv() => {
                let Some(p) = p else { return Ended::Lost(NO_CONNECTION) };
                match p.emsg {
                    crate::ipc::EMSG_GAME_CONNECT_TOKENS => {
                        if let Ok(m) = p.body() { auth.lock().unwrap().add_tokens(&m); }
                    }
                    crate::ipc::EMSG_AUTH_LIST_ACK => {
                        if let Ok(m) = p.body() { auth.lock().unwrap().on_ack(&m); }
                    }
                    crate::ipc::EMSG_TICKET_AUTH_COMPLETE => {
                        if let Ok(m) = p.body() { auth.lock().unwrap().on_auth_complete(&m); }
                    }
                    crate::ipc::EMSG_P2P_CONNECTION_INFO => {
                        if let Ok(m) = p.body::<CMsgClientP2pConnectionInfo>()
                            && m.rendezvous.as_ref().is_some_and(|r| !r.is_empty())
                        {
                            push.push(GameEvent::GsRendezvous { from: m.steam_id_src(), data: m.rendezvous().to_vec() });
                        }
                    }
                    EMSG_P2P_CONNECTION_FAIL => {
                        if let Ok(m) = p.body::<CMsgClientP2pConnectionFailInfo>() {
                            tracing::info!("game server {app_id}: Steam couldn't deliver a signal to {} (reason {})", m.steam_id_src(), m.ep2p_session_error());
                        }
                    }
                    emsg::CLIENT_LOGGED_OFF => {
                        let code = p.body::<CMsgClientLoggedOff>().map(|m| m.eresult()).unwrap_or(NO_CONNECTION);
                        tracing::info!("game server {app_id}: Steam signed it off ({code})");
                        return Ended::Lost(code);
                    }
                    _ => {}
                }
            }
            c = cmds.recv() => {
                let Some(c) = c else { return Ended::LoggedOff };
                match c {
                    Cmd::Update(new) => {
                        let (type_changed, players_changed) = (server_type(&new) != server_type(info), new.players != info.players);
                        *info = *new;
                        if type_changed {
                            let _ = conn.send(EMSG_GS_SERVER_TYPE, &server_type(info));
                        }
                        if players_changed {
                            let _ = conn.send(EMSG_GS_PLAYER_LIST, &player_list(info));
                        }
                        if info.active {
                            let _ = conn.send(EMSG_GAME_SERVER_UPDATE, &listing(info));
                        } else if listed {
                            let _ = conn.send(EMSG_GAME_SERVER_REMOVE, &CMsgGameServerRemove::default());
                        }
                        listed = info.active;
                    }
                    Cmd::BeginAuth { steam_id, ticket, reply } => {
                        let (conn, auth) = (conn.clone(), auth.clone());
                        tokio::spawn(async move {
                            let (response, owner) = crate::ipc::begin_auth(&conn, &auth, app_id, pid, steam_id, &ticket).await;
                            let _ = reply.send(Reply::AuthResult { response, owner });
                        });
                    }
                    Cmd::EndAuth { steam_id } => crate::ipc::end_auth(&conn, &auth, app_id, steam_id),
                    Cmd::UserStats { steam_id, reply } => {
                        let conn = conn.clone();
                        tokio::spawn(async move {
                            let result = crate::ipc::user_stats(&conn, app_id, steam_id, "english").await;
                            let _ = reply.send(Reply::Stats { result });
                        });
                    }
                    Cmd::StoreStats { steam_id, stats, crc, reply } => {
                        let conn = conn.clone();
                        tokio::spawn(async move {
                            let (eresult, crc) = crate::ipc::store_stats_as(&conn, app_id, me, steam_id, &stats, crc, false).await;
                            let _ = reply.send(Reply::Stored { eresult, crc });
                        });
                    }
                    Cmd::Cert { public_key, reply } => {
                        let conn = conn.clone();
                        tokio::spawn(async move {
                            let _ = reply.send(crate::ipc::networking_cert(&conn, app_id, &public_key).await);
                        });
                    }
                    Cmd::Rendezvous { to, data } => crate::ipc::rendezvous(&conn, app_id, me, to, &data),
                    Cmd::LogOff => {
                        if listed {
                            let _ = conn.send(EMSG_GAME_SERVER_REMOVE, &CMsgGameServerRemove::default());
                        }
                        let _ = conn.send(emsg::CLIENT_LOG_OFF, &CMsgClientLogOff::default());
                        // Give the socket a moment to send them.
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        return Ended::LoggedOff;
                    }
                }
            }
        }
    }
}

/// Sign in, then stay signed in (reconnecting) until the game logs off.
async fn run(app_id: AppId, pid: u32, token: String, mut info: GsInfo, mut cmds: mpsc::UnboundedReceiver<Cmd>, push: Pusher, first: oneshot::Sender<Reply>) {
    // The game's request gives up after 40 s: answer before that.
    let first_try = tokio::time::timeout(Duration::from_secs(35), sign_in(app_id, &token)).await.unwrap_or(Err(NO_CONNECTION));
    let signed = match first_try {
        Ok(s) => s,
        Err(code) => {
            let _ = first.send(Reply::GsLoggedOn { eresult: code, steam_id: 0, public_ip: 0 });
            while let Ok(c) = cmds.try_recv() {
                c.refuse();
            }
            return;
        }
    };
    let _ = first.send(Reply::GsLoggedOn { eresult: eresult::OK, steam_id: signed.steam_id, public_ip: signed.public_ip });
    let mut current = Some(signed);
    let mut wait = Duration::from_secs(2);
    loop {
        if let Some(s) = current.take() {
            match serve(s, app_id, pid, &mut info, &mut cmds, &push).await {
                Ended::LoggedOff => return,
                Ended::Lost(code) => push.push(GameEvent::GsStatus { connected: false, eresult: code, steam_id: 0, public_ip: 0 }),
            }
            wait = Duration::from_secs(2);
        }
        // Not connected: wait, keep the latest details, answer what can't run.
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                c = cmds.recv() => match c {
                    None | Some(Cmd::LogOff) => return,
                    Some(Cmd::Update(new)) => info = *new,
                    Some(other) => other.refuse(),
                },
            }
        }
        wait = (wait * 2).min(Duration::from_secs(60));
        if let Ok(s) = sign_in(app_id, &token).await {
            push.push(GameEvent::GsStatus { connected: true, eresult: eresult::OK, steam_id: s.steam_id, public_ip: s.public_ip });
            current = Some(s);
        }
    }
}

// -------------------------------------------------------- all servers

struct Host {
    cmds: mpsc::UnboundedSender<Cmd>,
    task: tokio::task::AbortHandle,
}

/// The game servers running now, by launch.
#[derive(Default)]
pub struct Hosts {
    by_launch: HashMap<String, Host>,
}

impl Drop for Hosts {
    fn drop(&mut self) {
        for h in self.by_launch.values() {
            h.task.abort();
        }
    }
}

impl Hosts {
    /// A game closed: its server signs off.
    pub fn stop(&mut self, launch: &str) {
        if let Some(h) = self.by_launch.remove(launch) {
            let _ = h.cmds.send(Cmd::LogOff);
        }
    }

    /// Answer a game server request from the game.
    pub fn call(&mut self, ipc: &Server, app_id: AppId, launch: &str, pid: u32, req: Request, reply: oneshot::Sender<Reply>) {
        self.by_launch.retain(|_, h| !h.task.is_finished());
        if let Request::GsLogOn { token, info } = req {
            self.stop(launch);
            let (cmds, rx) = mpsc::unbounded_channel();
            let task = tokio::spawn(run(app_id, pid, token, *info, rx, ipc.pusher(launch), reply)).abort_handle();
            self.by_launch.insert(launch.to_string(), Host { cmds, task });
            return;
        }
        let Some(host) = self.by_launch.get(launch) else {
            refused(req, reply);
            return;
        };
        let (cmd, reply) = match req {
            Request::GsLogOff => {
                self.stop(launch);
                let _ = reply.send(Reply::Ok);
                return;
            }
            Request::GsUpdate(info) => (Cmd::Update(info), Some(reply)),
            Request::GsEndAuth { steam_id } => (Cmd::EndAuth { steam_id }, Some(reply)),
            Request::GsRendezvous { to, data } => (Cmd::Rendezvous { to, data }, Some(reply)),
            Request::GsBeginAuth { steam_id, ticket } => (Cmd::BeginAuth { steam_id, ticket, reply }, None),
            Request::GsUserStats { steam_id } => (Cmd::UserStats { steam_id, reply }, None),
            Request::GsStoreStats { steam_id, stats, crc } => (Cmd::StoreStats { steam_id, stats, crc, reply }, None),
            Request::GsNetworkingCert { public_key } => (Cmd::Cert { public_key, reply }, None),
            other => {
                refused(other, reply);
                return;
            }
        };
        if let Err(e) = host.cmds.send(cmd) {
            e.0.refuse();
        }
        if let Some(r) = reply {
            let _ = r.send(Reply::Ok);
        }
    }
}

/// The answer to a request for a server that isn't signed in.
fn refused(req: Request, reply: oneshot::Sender<Reply>) {
    let r = match req {
        Request::GsBeginAuth { .. } => Reply::AuthResult { response: 1, owner: 0 },
        Request::GsUserStats { .. } => Reply::Stats { result: Err("not connected".into()) },
        Request::GsStoreStats { crc, .. } => Reply::Stored { eresult: NO_CONNECTION, crc },
        Request::GsNetworkingCert { .. } => Reply::Err { message: "The game server isn't signed in".into() },
        _ => Reply::Ok,
    };
    let _ = reply.send(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> GsInfo {
        GsInfo {
            app_id: 480,
            game_port: 27015,
            query_port: 27016,
            version: "1.0.0.0".into(),
            mod_dir: "spacewar".into(),
            name: "Test".into(),
            map: "arena".into(),
            max_players: 8,
            bots: 2,
            players: vec![76561197960287930],
            password: true,
            dedicated: true,
            active: true,
            ..Default::default()
        }
    }

    #[test]
    fn signs_in_as_the_right_kind_of_server() {
        // Anonymous game server / game server, public universe.
        assert_eq!(logon_steam_id(""), 0x0140_0000_0000_0000);
        assert_eq!(logon_steam_id("TOKEN"), 0x0130_0000_0000_0000);
        let (id, m) = logon_message(480, "");
        assert_eq!((id, m.game_server_app_id, m.game_server_token), (emsg::CLIENT_LOGON, Some(480), None));
        let (id, m) = logon_message(480, "TOKEN");
        assert_eq!((id, m.game_server_token.as_deref()), (EMSG_LOGON_GAME_SERVER, Some("TOKEN")));
    }

    #[test]
    fn listing_carries_the_details_and_is_never_vac_secure() {
        let l = listing(&info());
        assert_eq!((l.app_id(), l.query_port(), l.game_port(), l.max_players(), l.bot_count()), (480, 27016, 27015, 8, 2));
        assert_eq!((l.server_name(), l.map(), l.gamedir()), ("Test", "arena", "spacewar"));
        assert_eq!(l.players.len(), 1);
        assert_eq!(l.secure, Some(false));
        // Active, dedicated and passworded, without the secure bit.
        assert_eq!(server_flags(&info()), 1 | 4 | 16);
        assert_eq!(server_flags(&info()) & 2, 0);
    }

    #[tokio::test]
    async fn requests_without_a_signed_in_server_are_answered() {
        let (ipc, _rx) = Server::start();
        let mut hosts = Hosts::default();
        let (tx, rx) = oneshot::channel();
        hosts.call(&ipc, 480, "launch", 1, Request::GsBeginAuth { steam_id: 5, ticket: vec![1] }, tx);
        assert!(matches!(rx.await, Ok(Reply::AuthResult { response: 1, owner: 0 })));
        let (tx, rx) = oneshot::channel();
        hosts.call(&ipc, 480, "launch", 1, Request::GsStoreStats { steam_id: 5, stats: vec![], crc: 9 }, tx);
        assert!(matches!(rx.await, Ok(Reply::Stored { eresult: NO_CONNECTION, crc: 9 })));
    }
}

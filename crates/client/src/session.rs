//! The engine loop: idle -> connect -> authenticate -> logon -> online -> idle.

use crate::{Command, Error, Event, Link, Sink, launch, offline, pics, store};
use auth::{Device, Guard, PendingAuth};
use model::{AppId, OwnedApp, Persona, PersonaState, Relationship, SteamId};
use net::{Connection, Packet, emsg, eresult};
use proto::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

const PROTOCOL_VERSION: u32 = 65580;
const OS_TYPE_WIN11: u32 = 20;
/// Status | PlayerName | Presence | LastSeen | GameExtraInfo | RichPresence
const PERSONA_FLAGS: u32 = 1 | 2 | 16 | 64 | 256 | 4096;

type Cmds = mpsc::UnboundedReceiver<Command>;

enum Login {
    Password { account: String, password: String },
    Qr,
    Saved(store::SavedAccount),
    /// Offline mode from the cached sign-in.
    Offline(store::SavedAccount),
}

pub async fn run(mut cmds: Cmds, sink: Sink) {
    // A login command that arrived while another login was waiting.
    let mut next: Option<Command> = None;
    loop {
        let cmd = match next.take() {
            Some(c) => c,
            None => match cmds.recv().await {
                Some(c) => c,
                None => return,
            },
        };
        let login = match cmd {
            Command::LoginPassword { account, password } => Login::Password { account, password },
            Command::LoginQr => Login::Qr,
            Command::LoginSaved { account } => match store::find(&account) {
                Some(saved) => Login::Saved(saved),
                None => {
                    sink(Event::LoginFailed(format!("No saved login for {account}")));
                    continue;
                }
            },
            Command::StartOffline { account } => match store::find(&account) {
                Some(saved) => Login::Offline(saved),
                None => {
                    sink(Event::LoginFailed(format!("No saved login for {account}")));
                    continue;
                }
            },
            Command::ForgetAccount { account } => {
                store::remove(&account);
                continue;
            }
            _ => continue,
        };
        // A saved login that can't reach Steam isn't a failed login: offer
        // offline mode instead, like Steam's "Connection error" window.
        let saved = match &login {
            Login::Saved(a) => Some(a.clone()),
            _ => None,
        };
        sink(Event::Connecting);
        match session(login, &mut cmds, &sink, &mut next).await {
            Ok(reason) => sink(Event::LoggedOut { reason }),
            Err(_) if next.is_some() => {} // replaced by a newer login
            Err(e) if saved.is_some() && is_network(&e) => {
                let Some(a) = saved else { continue };
                tracing::warn!("can't reach Steam for {}: {e}", a.account_name);
                let offline_ready = offline::load_account(a.steam_id).is_some();
                sink(Event::ConnectionError { account: a.account_name, message: e.to_string(), offline_ready });
            }
            Err(e) => sink(Event::LoginFailed(e.to_string())),
        }
    }
}

async fn connect_any() -> Result<(Connection, net::connection::Events), Error> {
    let servers = net::directory::websocket_servers().await?;
    let mut last = None;
    for server in servers.iter().take(4) {
        match tokio::time::timeout(Duration::from_secs(10), Connection::connect(server)).await {
            Ok(Ok(c)) => return Ok(c),
            Ok(Err(e)) => {
                tracing::warn!("{server}: {e}");
                last = Some(e);
            }
            Err(_) => {
                tracing::warn!("{server}: no answer");
                last = Some(net::Error::Timeout);
            }
        }
    }
    Err(last.unwrap_or(net::Error::NoServers).into())
}

/// Returns `Ok(reason)` once a logged-in session ends, `Err` if login failed.
/// The network failed (no connection, timeout, Steam down), as opposed to
/// Steam saying no to the login.
fn is_network(e: &Error) -> bool {
    match e {
        Error::Net(net::Error::EResult(code, _)) | Error::Auth(auth::Error::Net(net::Error::EResult(code, _))) => {
            // NoConnection, Timeout, ServiceUnavailable, TryAnotherCM.
            matches!(*code, 3 | 16 | eresult::SERVICE_UNAVAILABLE | eresult::TRY_ANOTHER_CM)
        }
        Error::Net(net::Error::Decode(_) | net::Error::Truncated) => false,
        Error::Net(_) | Error::Auth(auth::Error::Net(_)) => true,
        _ => false,
    }
}

/// Worth trying again later (while reconnecting): network trouble, or
/// Steam being busy. Anything else means the saved login is no good.
fn is_transient(e: &Error) -> bool {
    is_network(e) || matches!(e, Error::Net(net::Error::EResult(code, _)) if matches!(*code, eresult::FAIL | eresult::RATE_LIMIT_EXCEEDED | eresult::LOGIN_THROTTLED))
}

/// What a successful `ClientLogon` told us.
#[derive(Clone, Copy)]
struct Logon {
    steam_id: SteamId,
    heartbeat: i32,
    cell_id: u32,
    public_ip: u32,
}

/// `ClientLogon` with the saved refresh token, on a fresh connection.
async fn logon(conn: &Connection, events: &mut net::connection::Events, account: &store::SavedAccount) -> Result<Logon, Error> {
    let device = Device::default();
    conn.set_session(account.steam_id, 0);
    conn.send(
        emsg::CLIENT_LOGON,
        &CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            client_os_type: Some(OS_TYPE_WIN11),
            client_language: Some("english".into()),
            should_remember_password: Some(true),
            account_name: Some(account.account_name.clone()),
            access_token: Some(account.refresh_token.clone()),
            machine_name: Some(device.name.clone()),
            chat_mode: Some(2),
            // Steam tells sessions on one account apart by this id: a second
            // sign-in with the same id replaces the first (error 34). Each run
            // of the app gets its own, like separate PCs; reconnecting keeps it,
            // so the new session replaces only our own old one.
            obfuscated_private_ip: Some(CMsgIpAddress {
                ip: Some(c_msg_ip_address::Ip::V4(logon_id() ^ 0xBAAD_F00D)),
            }),
            ..Default::default()
        },
    )?;
    let reply = wait_for(events, emsg::CLIENT_LOGON_RESPONSE).await?;
    let resp: CMsgClientLogonResponse = reply.body()?;
    if resp.eresult() != eresult::OK {
        if matches!(resp.eresult(), eresult::ACCESS_DENIED | eresult::INVALID_PASSWORD | eresult::EXPIRED) {
            store::remove(&account.account_name);
        }
        return Err(net::Error::EResult(resp.eresult(), String::new()).into());
    }
    let steam_id = reply.header.steamid.filter(|&s| s != 0).unwrap_or(account.steam_id);
    conn.set_session(steam_id, reply.header.client_sessionid());
    let public_ip = match resp.public_ip.as_ref().and_then(|ip| ip.ip.as_ref()) {
        Some(c_msg_ip_address::Ip::V4(v)) => *v,
        _ => 0,
    };
    Ok(Logon { steam_id, heartbeat: resp.heartbeat_seconds(), cell_id: resp.cell_id(), public_ip })
}

/// A new connection and sign-in for a session that lost its connection.
async fn reconnect(account: store::SavedAccount) -> Result<(Connection, net::connection::Events, Logon), Error> {
    let (conn, mut events) = connect_any().await?;
    conn.send(emsg::CLIENT_HELLO, &CMsgClientHello { protocol_version: Some(PROTOCOL_VERSION) })?;
    let info = logon(&conn, &mut events, &account).await?;
    Ok((conn, events, info))
}

async fn session(login: Login, cmds: &mut Cmds, sink: &Sink, next: &mut Option<Command>) -> Result<String, Error> {
    if let Login::Offline(account) = &login {
        if offline::load_account(account.steam_id).is_none() {
            return Err(Error::Other("This account hasn't signed in online on this PC yet, so it can't start in offline mode.".into()));
        }
        return online(None, cmds, sink, account).await;
    }
    let (conn, mut events) = connect_any().await?;
    conn.send(emsg::CLIENT_HELLO, &CMsgClientHello { protocol_version: Some(PROTOCOL_VERSION) })?;
    let device = Device::default();

    let account = match login {
        Login::Saved(saved) | Login::Offline(saved) => saved,
        Login::Qr => {
            let mut pending = auth::begin_qr(&conn, &device).await?;
            let Some(tokens) = guard_flow(&conn, &mut pending, cmds, sink, next).await? else {
                return Err(Error::Other("Login cancelled".into()));
            };
            store::SavedAccount {
                account_name: tokens.account_name,
                steam_id: tokens.steam_id,
                persona_name: String::new(),
                avatar_hash: None,
                refresh_token: tokens.refresh_token,
                guard_data: tokens.guard_data,
            }
        }
        Login::Password { account, password } => {
            let guard_data = store::find(&account).and_then(|a| a.guard_data);
            let mut pending = auth::begin(&conn, &account, &password, guard_data, &device).await?;
            drop(password);
            let Some(tokens) = guard_flow(&conn, &mut pending, cmds, sink, next).await? else {
                return Err(Error::Other("Login cancelled".into()));
            };
            store::SavedAccount {
                account_name: if tokens.account_name.is_empty() { account } else { tokens.account_name },
                steam_id: tokens.steam_id,
                persona_name: String::new(),
                avatar_hash: None,
                refresh_token: tokens.refresh_token,
                guard_data: tokens.guard_data,
            }
        }
    };

    let info = logon(&conn, &mut events, &account).await?;
    store::upsert(account.clone());
    sink(Event::LoggedIn { steam_id: info.steam_id, account: account.account_name.clone() });
    online(Some((conn, events, info)), cmds, sink, &account).await
}

async fn guard_flow(
    conn: &Connection,
    pending: &mut PendingAuth,
    cmds: &mut Cmds,
    sink: &Sink,
    next: &mut Option<Command>,
) -> Result<Option<auth::Tokens>, Error> {
    let mut shown_url = pending.challenge_url.clone();
    if let Some(url) = &shown_url {
        sink(Event::QrChallenge { url: url.clone() });
    } else if pending.preferred() != Guard::None {
        sink(Event::GuardRequired { guards: pending.guards.clone() });
    }
    let mut tick = tokio::time::interval(pending.interval);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Some(t) = pending.poll(conn).await? {
                    return Ok(Some(t));
                }
                if pending.challenge_url != shown_url {
                    shown_url = pending.challenge_url.clone();
                    if let Some(url) = &shown_url {
                        sink(Event::QrChallenge { url: url.clone() });
                    }
                }
            }
            cmd = cmds.recv() => match cmd {
                Some(Command::SubmitCode(code)) => match pending.submit_code(conn, &code).await {
                    Ok(()) => {
                        if let Some(t) = pending.poll(conn).await? {
                            return Ok(Some(t));
                        }
                    }
                    Err(e) => sink(Event::GuardCodeRejected(e.to_string())),
                },
                Some(Command::CancelLogin) | None => return Ok(None),
                Some(c @ (Command::LoginPassword { .. } | Command::LoginSaved { .. } | Command::LoginQr)) => {
                    *next = Some(c);
                    return Ok(None);
                }
                Some(_) => {}
            }
        }
    }
}

async fn wait_for(events: &mut net::connection::Events, want: u32) -> Result<Packet, Error> {
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            p = events.recv() => match p {
                Some(p) if p.emsg == want => return Ok(p),
                Some(p) if p.emsg == emsg::CLIENT_LOGGED_OFF => {
                    let off: CMsgClientLoggedOff = p.body()?;
                    return Err(net::Error::EResult(off.eresult(), String::new()).into());
                }
                Some(_) => {}
                None => return Err(net::Error::Disconnected.into()),
            },
            _ = &mut deadline => return Err(net::Error::Timeout.into()),
        }
    }
}

/// Data the online loop keeps between messages.
struct Online {
    conn: Connection,
    me: SteamId,
    account: String,
    friends: HashMap<SteamId, Relationship>,
    personas: HashMap<SteamId, Persona>,
    apps: HashMap<AppId, OwnedApp>,
    playing: Option<AppId>,
    /// Process ids of games we started.
    running: HashMap<AppId, u32>,
    cell_id: u32,
    /// Content host the player picked (a partner CDN), tried first.
    download_host: Option<String>,
    /// Games whose names we've looked up (not in the library).
    names_asked: std::collections::HashSet<AppId>,
    /// Download region the player picked (overrides the login's region).
    download_cell: Option<u32>,
    /// Steam Cloud: where `userdata` is, and games the player turned it off for.
    steam_root: Option<std::path::PathBuf>,
    cloud_off: std::collections::HashSet<AppId>,
    /// Install folders of games we started (for the upload after they close).
    launched_dirs: HashMap<AppId, std::path::PathBuf>,
    queue: std::collections::VecDeque<crate::download::InstallRequest>,
    active: Option<(AppId, std::sync::Arc<depot::Progress>)>,
    /// The running download's request, kept so it can be saved and resumed.
    active_req: Option<crate::download::InstallRequest>,
    /// Unfinished downloads from the last run were put back in the queue.
    downloads_restored: bool,
    limit_kbps: u64,
    concurrency: u32,
    refresh_token: String,
    /// Short-lived web token for steamcommunity.com (memory only).
    web: crate::market::WebCache,
    /// Pipe that games with our steamclient DLL talk to.
    ipc: crate::ipc::Server,
    /// Each running game's token on that pipe.
    launch_tokens: HashMap<AppId, String>,
    /// Games the official Steam client runs (anti-cheat), by install folder.
    official: HashMap<AppId, std::path::PathBuf>,
    /// Two-letter country of this connection.
    country: String,
    /// The wallet's Steam currency code, once known (prices and cart country).
    wallet_currency: Option<i32>,
    /// Auth session tickets for games with our steamclient.
    auth: crate::ipc::SharedAuth,
    /// Online, offline mode or reconnecting.
    link: Link,
    /// The player wants to be online (false after "Go offline").
    want_online: bool,
    /// The reconnect running now was a "Go online" from offline mode.
    from_offline: bool,
    /// Bumped for every new connection (tells stale checks apart).
    generation: u64,
    /// When Steam last sent anything (to spot a connection gone silent).
    last_rx: std::time::Instant,
    probing: bool,
    /// The status the player chose (set again after reconnecting).
    persona_state: PersonaState,
    /// Games running while not connected: (start, in offline mode).
    unconnected_play: HashMap<AppId, (u64, bool)>,
    /// The running download was paused because the connection dropped.
    auto_paused: bool,
    /// The offline cache needs writing; when it was last written.
    cache_dirty: bool,
    cache_saved: std::time::Instant,
    /// Games whose pending Steam Cloud upload is running.
    cloud_flushing: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<AppId>>>,
}

/// Seconds before reconnect attempt `n` (1, 2, …), like Steam: quick at
/// first, then at most once a minute.
fn backoff(attempt: u32) -> u32 {
    match attempt {
        0 | 1 => 2,
        n => (5u32 << (n - 2).min(4)).min(60),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A packet stream that never delivers anything (while not connected).
fn quiet_events() -> (mpsc::UnboundedSender<Packet>, net::connection::Events) {
    mpsc::unbounded_channel()
}

/// How a server message ends the session.
enum Ended {
    /// Signed out for good (logged off, replaced): back to sign-in.
    Logout(String),
    /// Steam dropped us for a reason that passes (maintenance, busy server).
    Dropped,
}

/// The signed-in session, connected or not. `link` is `None` for offline
/// mode: everything comes from the cache saved at the last sign-in. A lost
/// connection doesn't end it: running games and the UI carry on while it
/// reconnects in the background.
async fn online(
    link: Option<(Connection, net::connection::Events, Logon)>,
    cmds: &mut Cmds,
    sink: &Sink,
    account: &store::SavedAccount,
) -> Result<String, Error> {
    launch::repair_registry();
    let (ipc_server, mut ipc_rx) = crate::ipc::Server::start();
    // Put Valve's steamclient back in the registry however this session ends.
    struct RestoreRegistry;
    impl Drop for RestoreRegistry {
        fn drop(&mut self) {
            launch::swap_restore_now();
        }
    }
    let _restore = RestoreRegistry;
    let offline_start = link.is_none();
    // Keeps the quiet stream open while not connected.
    let mut quiet_tx: Option<mpsc::UnboundedSender<Packet>> = None;
    let (conn, mut events, info) = match link {
        Some(l) => l,
        None => {
            let (tx, rx) = quiet_events();
            quiet_tx = Some(tx);
            (Connection::closed(), rx, Logon { steam_id: account.steam_id, heartbeat: 9, cell_id: 0, public_ip: 0 })
        }
    };
    let me = info.steam_id;
    let mut st = Online {
        conn: conn.clone(),
        me,
        account: account.account_name.clone(),
        friends: HashMap::new(),
        personas: HashMap::new(),
        apps: HashMap::new(),
        playing: None,
        running: HashMap::new(),
        cell_id: info.cell_id,
        download_cell: None,
        names_asked: Default::default(),
        download_host: None,
        steam_root: None,
        cloud_off: Default::default(),
        launched_dirs: HashMap::new(),
        queue: Default::default(),
        active_req: None,
        downloads_restored: false,
        active: None,
        limit_kbps: 0,
        concurrency: 16,
        refresh_token: account.refresh_token.clone(),
        web: Default::default(),
        ipc: ipc_server,
        launch_tokens: HashMap::new(),
        official: HashMap::new(),
        country: String::new(),
        wallet_currency: None,
        auth: crate::ipc::Auth::new(info.public_ip),
        link: if offline_start { Link::Offline } else { Link::Online },
        want_online: !offline_start,
        generation: 0,
        last_rx: std::time::Instant::now(),
        probing: false,
        persona_state: PersonaState::Online,
        unconnected_play: HashMap::new(),
        auto_paused: false,
        cache_dirty: false,
        cache_saved: std::time::Instant::now(),
        cloud_flushing: Default::default(),
        from_offline: false,
    };
    let (dl_tx, mut dl_rx) = mpsc::unbounded_channel::<(AppId, Result<String, String>)>();
    let (apps_tx, mut apps_rx) = mpsc::unbounded_channel::<Vec<OwnedApp>>();
    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<AppId>();
    // Launches that waited for a Steam Cloud download.
    let (launch_tx, mut launch_rx) = mpsc::unbounded_channel::<(AppId, std::path::PathBuf, String)>();
    // Reconnect attempts, and checks of a connection that went quiet.
    let (rc_tx, mut rc_rx) = mpsc::unbounded_channel::<Result<(Connection, net::connection::Events, Logon), Error>>();
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel::<(u64, bool)>();
    let (flush_tx, mut flush_rx) = mpsc::unbounded_channel::<()>();
    let mut retry_at: Option<tokio::time::Instant> = None;
    let mut connecting = false;
    let mut hb = tokio::time::interval(Duration::from_secs(info.heartbeat.clamp(5, 60) as u64));
    hb.tick().await;

    if offline_start {
        match offline::load_account(me) {
            Some(cache) => st.start_offline(cache, sink),
            None => return Err(Error::Other("No offline data for this account".into())),
        }
    } else {
        sink(Event::Link(Link::Online));
        st.connected(sink);
        // Play time and stats from an earlier offline run go to Steam now.
        st.flush_offline(sink);
    }
    st.send_offline_lists(sink);

    // The connection is gone: carry on without it and try again soon.
    macro_rules! lost {
        ($why:expr) => {{
            tracing::warn!("lost the connection to Steam: {}", $why);
            let (tx, rx) = st.drop_connection(sink, false);
            events = rx;
            quiet_tx = Some(tx);
            retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(backoff(1) as u64));
        }};
    }

    loop {
        let conn = st.conn.clone();
        tokio::select! {
            p = events.recv() => match p {
                Some(p) => {
                    st.last_rx = std::time::Instant::now();
                    match st.handle(p, sink, &apps_tx) {
                        Ok(None) => {}
                        Ok(Some(Ended::Logout(reason))) => return Ok(reason),
                        Ok(Some(Ended::Dropped)) => lost!("Steam ended the session for now"),
                        Err(e) => tracing::warn!("bad message from Steam: {e}"),
                    }
                }
                None => lost!("the server closed the connection"),
            },
            cmd = cmds.recv() => match cmd {
                None => return Ok("Shutting down".into()),
                Some(Command::LogOut) => {
                    let _ = conn.send(emsg::CLIENT_LOG_OFF, &CMsgClientLogOff {});
                    return Ok("Logged out".into());
                }
                Some(Command::GoOnline) => {
                    if st.link != Link::Online && !connecting {
                        st.from_offline = st.link == Link::Offline;
                        st.want_online = true;
                        retry_at = Some(tokio::time::Instant::now());
                    }
                }
                Some(Command::GoOffline) => {
                    st.want_online = false;
                    retry_at = None;
                    if st.link == Link::Online {
                        let _ = conn.send(emsg::CLIENT_LOG_OFF, &CMsgClientLogOff {});
                        let (tx, rx) = st.drop_connection(sink, true);
                        events = rx;
                        quiet_tx = Some(tx);
                    } else {
                        st.set_link(Link::Offline, sink);
                    }
                }
                Some(Command::SetPersonaState(s)) => {
                    st.persona_state = s;
                    let _ = set_persona(&conn, s);
                }
                Some(Command::Launch { app_id, install_dir, args }) => {
                    // Like Steam: bring down newer cloud saves first (games the
                    // official client runs get its own cloud sync instead).
                    let official = st.apps.get(&app_id).is_some_and(|a| matches!(launch::mode(a, &install_dir), crate::LaunchMode::OfficialSteam { .. }));
                    match st.cloud_target(app_id, &install_dir).filter(|_| !official) {
                        // Offline: play with the saves on this PC; they go up once online.
                        Some(_) if st.link != Link::Online && !st.running.contains_key(&app_id) => {
                            sink(Event::CloudSkipped { app_id });
                            let _ = st.launch(app_id, &install_dir, &args, sink, &exit_tx);
                        }
                        Some(target) if !st.running.contains_key(&app_id) => {
                            sink(Event::CloudSyncing { app_id });
                            let (conn, sink, tx) = (conn.clone(), sink.clone(), launch_tx.clone());
                            tokio::spawn(async move {
                                let report = crate::cloud::sync(&conn, &target, crate::cloud::Mode::Download, &[], &[]).await;
                                let blocked = !report.conflicts.is_empty();
                                sink(Event::CloudReport { report, held_launch: blocked.then(|| args.clone()) });
                                if !blocked {
                                    let _ = tx.send((app_id, install_dir, args));
                                }
                            });
                        }
                        _ => {
                            let _ = st.launch(app_id, &install_dir, &args, sink, &exit_tx);
                        }
                    }
                }
                Some(Command::LaunchWithoutCloud { app_id, install_dir, args }) => {
                    let _ = st.launch(app_id, &install_dir, &args, sink, &exit_tx);
                }
                Some(Command::ProbeRegions) => {
                    let sink = sink.clone();
                    tokio::spawn(async move {
                        sink(Event::Regions(crate::regions::probe().await));
                    });
                }
                Some(Command::DownloadRegion { cell_id, host }) => {
                    st.download_cell = cell_id;
                    st.download_host = host;
                }
                Some(Command::CloudSettings { steam_root, disabled }) => {
                    st.steam_root = steam_root;
                    st.cloud_off = disabled.into_iter().collect();
                }
                Some(Command::CloudSync { app_id, install_dir, mode, keep_local, keep_cloud, then_launch }) => {
                    // A manual sync works even when automatic sync is off.
                    if let Some(target) = st.cloud_target_any(app_id, &install_dir) {
                        sink(Event::CloudSyncing { app_id });
                        let (conn, sink, tx, me) = (conn.clone(), sink.clone(), launch_tx.clone(), st.me);
                        tokio::spawn(async move {
                            let report = crate::cloud::sync(&conn, &target, mode, &keep_local, &keep_cloud).await;
                            let blocked = !report.conflicts.is_empty();
                            if matches!(mode, crate::cloud::Mode::Upload | crate::cloud::Mode::Sync) && report.errors.is_empty() && !blocked {
                                offline::remove_cloud_pending(me, app_id);
                                sink(Event::CloudPending(offline::cloud_pending(me).iter().map(|p| p.0).collect()));
                            }
                            sink(Event::CloudReport { report, held_launch: then_launch.clone().filter(|_| blocked) });
                            if let (Some(args), false) = (then_launch, blocked) {
                                let _ = tx.send((app_id, install_dir, args));
                            }
                        });
                    }
                }
                Some(Command::StopGame { app_id }) => {
                    if let Some(dir) = st.official.get(&app_id).cloned() {
                        tokio::task::spawn_blocking(move || launch::kill_in_folder(&dir));
                    } else if let Some(pid) = st.running.get(&app_id).copied() {
                        tokio::task::spawn_blocking(move || launch::kill_tree(pid));
                    }
                }
                Some(Command::ForgetAccount { account }) => store::remove(&account),
                Some(Command::SendMessage { to, text }) => st.send_message(to, text, sink),
                Some(Command::LoadHistory { friend }) => st.load_history(friend, sink),
                Some(Command::LoadProfile { steam_id }) => st.load_profile(steam_id, sink),
                Some(Command::AddFriend { steam_id }) => {
                    let _ = conn.send(emsg::CLIENT_ADD_FRIEND, &CMsgClientAddFriend { steamid_to_add: Some(steam_id), ..Default::default() });
                }
                Some(Command::RemoveFriend { steam_id }) => {
                    let _ = conn.send(emsg::CLIENT_REMOVE_FRIEND, &CMsgClientRemoveFriend { friendid: Some(steam_id) });
                }
                Some(Command::LoadInventory { steam_id, app_id, context_id, language }) => st.load_inventory(steam_id, app_id, context_id, language, sink),
                Some(Command::LoadInventoryApps { steam_id }) => st.load_inventory_apps(steam_id, sink),
                Some(Command::MarketPrice { app_id, hash_name, currency, country, full }) => st.market_price(app_id, hash_name, currency, country, full, sink),
                Some(Command::SellItem { app_id, context_id, asset_id, amount, receive }) => st.sell_item(app_id, context_id, asset_id, amount, receive, sink),
                Some(Command::LoadMyListings) => st.my_listings(sink),
                Some(Command::LoadSteamCollections) => {
                    let (conn, sink) = (conn.clone(), sink.clone());
                    tokio::spawn(async move {
                        let req = proto::webui::CCloudConfigStoreDownloadRequest { versions: vec![proto::webui::CCloudConfigStoreNamespaceVersion { enamespace: Some(1), version: Some(0) }] };
                        let result = conn
                            .call::<_, proto::webui::CCloudConfigStoreDownloadResponse>("CloudConfigStore.Download#1", &req)
                            .await
                            .map_err(|e| e.to_string())
                            .map(|r| crate::social::collections(r.data.iter().flat_map(|d| d.entries.iter()).map(|e| (e.key(), e.is_deleted(), e.value()))));
                        sink(Event::SteamCollections(result));
                    });
                }
                Some(Command::LoadComments { steam_id, start }) => st.with_web(sink, move |web, sink| async move {
                    let result = crate::social::comments(web.as_deref().ok(), steam_id, start).await;
                    sink(Event::Comments { steam_id, result });
                }),
                Some(Command::PostComment { steam_id, text }) => st.with_web(sink, move |web, sink| async move {
                    let result = match web {
                        Ok(w) => crate::social::post_comment(&w, steam_id, &text).await,
                        Err(e) => Err(e),
                    };
                    let ok = result.is_ok();
                    sink(Event::CommentPosted { steam_id, result: result.map(|_| ()) });
                    if ok {
                        let w = crate::market::Web::anonymous();
                        sink(Event::Comments { steam_id, result: crate::social::comments(Some(&w), steam_id, 0).await });
                    }
                }),
                Some(Command::DeleteComment { steam_id, comment_id }) => st.with_web(sink, move |web, sink| async move {
                    let result = match web {
                        Ok(w) => crate::social::delete_comment(&w, steam_id, comment_id).await,
                        Err(e) => Err(e),
                    };
                    match result {
                        Ok(page) => sink(Event::Comments { steam_id, result: Ok(page) }),
                        Err(e) => sink(Event::Error(format!("Couldn't delete the comment: {e}"))),
                    }
                }),
                Some(Command::LoadBadges { steam_id }) => st.with_web(sink, move |web, sink| async move {
                    let result = crate::social::badges(web.as_deref().ok(), steam_id).await;
                    sink(Event::Badges { steam_id, result });
                }),
                Some(Command::LoadNotifications) => st.load_notifications(sink),
                Some(Command::MarkNotificationsRead { ids }) => st.mark_read(ids, sink),
                Some(Command::LoadSupportTickets) => st.support_tickets(sink),
                Some(Command::LoadSupportTicket { id, url }) => st.support_ticket(id, url, sink),
                Some(Command::CancelListing { listing_id }) => st.cancel_listing(listing_id, sink),
                Some(Command::CancelBuyOrder { order_id }) => st.cancel_buy_order(order_id, sink),
                Some(Command::LoadPlayerGames { steam_id, language }) => st.load_player_games(steam_id, language, sink),
                Some(Command::LoadAchievementProgress { app_ids }) => st.load_achievement_progress(app_ids, sink),
                Some(Command::LoadAchievements { app_id, language }) => st.load_achievements(app_id, language, sink),
                Some(Command::ActivateKey { key }) => st.activate_key(key, sink),
                Some(Command::Install(req)) => {
                    if st.active.as_ref().map(|a| a.0) != Some(req.app_id) && !st.queue.iter().any(|q| q.app_id == req.app_id) {
                        st.queue.push_back(req);
                    }
                    st.pump(sink, &dl_tx);
                    st.save_downloads();
                }
                Some(Command::PauseDownload { app_id }) => {
                    st.auto_paused = false;
                    st.with_progress(app_id, |p| p.paused.store(true, std::sync::atomic::Ordering::Relaxed));
                }
                Some(Command::ResumeDownload { app_id }) => st.with_progress(app_id, |p| p.paused.store(false, std::sync::atomic::Ordering::Relaxed)),
                Some(Command::CancelDownload { app_id }) => {
                    st.queue.retain(|q| q.app_id != app_id);
                    st.with_progress(app_id, |p| p.cancel.store(true, std::sync::atomic::Ordering::Relaxed));
                    if st.active_req.as_ref().is_some_and(|r| r.app_id == app_id) {
                        st.active_req = None;
                    }
                    st.save_downloads();
                    sink(Event::DownloadQueue(st.queue.iter().map(|q| q.app_id).collect()));
                }
                Some(Command::DownloadLimits { limit_kbps, concurrency }) => {
                    st.limit_kbps = limit_kbps;
                    st.concurrency = concurrency.clamp(1, 64);
                    if let Some((_, p)) = &st.active {
                        p.limit_kbps.store(limit_kbps, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Some(Command::UnsubscribeWorkshop { app_id, file_id }) => st.unsubscribe(app_id, file_id, sink),
                Some(Command::WorkshopQuery { request_id, app_id, query_type, page, search, days, tags }) => {
                    st.workshop_query(request_id, app_id, query_type, page, search, days, tags, sink)
                }
                Some(Command::WorkshopSubscribe { app_id, file_id }) => st.workshop_subscribe(app_id, file_id, sink),
                Some(Command::LoadWorkshopTags { app_id, language }) => {
                    let sink = sink.clone();
                    tokio::spawn(async move {
                        let result = crate::social::workshop_tags(app_id, &language).await;
                        sink(Event::WorkshopTags { app_id, result });
                    });
                }
                Some(Command::LoadWishlist) => st.load_wishlist(sink),
                Some(Command::WishlistAdd { app_id }) => st.wishlist_change(app_id, true, sink),
                Some(Command::WishlistRemove { app_id }) => st.wishlist_change(app_id, false, sink),
                Some(Command::LoadCart) => st.load_cart(sink),
                Some(Command::CartAdd { package_id, gift_to }) => st.cart_add(package_id, gift_to, sink),
                Some(Command::CartRemove { line_item_id }) => st.cart_remove(line_item_id, sink),
                Some(Command::Hub(req)) => {
                    let (conn, me) = (conn.clone(), st.me);
                    st.with_web(sink, move |web, sink| crate::hub::handle(conn, me, web, req, sink));
                }
                Some(Command::Typing { to }) => {
                    let conn = conn.clone();
                    tokio::spawn(async move {
                        let _: Result<CFriendMessagesSendMessageResponse, _> = conn
                            .call("FriendMessages.SendMessage#1", &CFriendMessagesSendMessageRequest {
                                steamid: Some(to), chat_entry_type: Some(2), ..Default::default()
                            })
                            .await;
                    });
                }
                Some(other) => tracing::debug!("ignored while signed in: {other:?}"),
            },
            Some(apps) = apps_rx.recv() => {
                st.apps = apps.iter().map(|a| (a.app_id, a.clone())).collect();
                sink(Event::Apps(apps));
                // Downloads that were running or queued when the app closed
                // carry on (files already on disk are checked and kept).
                if !st.downloads_restored {
                    st.downloads_restored = true;
                    st.restore_downloads(sink, &dl_tx);
                }
                st.cache_dirty = true;
                st.save_cache();
                // Saves changed offline go up once the library has sent its
                // cloud settings for these games (a moment later).
                let tx = flush_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let _ = tx.send(());
                });
            }
            Some(()) = flush_rx.recv() => st.flush_cloud(sink),
            Some((app_id, result)) = dl_rx.recv() => {
                st.active = None;
                st.active_req = None;
                st.auto_paused = false;
                sink(Event::DownloadFinished { app_id, result });
                st.pump(sink, &dl_tx);
                st.save_downloads();
            }
            Some((app_id, dir, args)) = launch_rx.recv() => {
                let _ = st.launch(app_id, &dir, &args, sink, &exit_tx);
            }
            Some(call) = ipc_rx.recv() => st.ipc_call(call, sink),
            Some(app_id) = exit_rx.recv() => {
                st.running.remove(&app_id);
                st.official.remove(&app_id);
                if let Some(token) = st.launch_tokens.remove(&app_id) {
                    launch::swap_exited(&token);
                    st.ipc.forget(&token);
                    crate::ipc::game_closed(&conn, &st.auth, app_id);
                }
                // Played while not connected: keep the time here (Steam never saw it).
                if let Some((start, offline_mode)) = st.unconnected_play.remove(&app_id) {
                    offline::record_play(st.me, offline::PlaySession { app_id, start, seconds: unix_now().saturating_sub(start), offline: offline_mode, reported: false });
                    st.send_offline_lists(sink);
                }
                // Like Steam: send changed saves up after the game closes,
                // or remember to once back online.
                if let Some(dir) = st.launched_dirs.remove(&app_id) {
                    if let Some(target) = st.cloud_target(app_id, &dir) {
                        if st.link == Link::Online {
                            st.upload_saves(app_id, target, sink);
                        } else {
                            offline::add_cloud_pending(st.me, app_id, &dir);
                            st.send_offline_lists(sink);
                        }
                    }
                }
                if st.playing == Some(app_id) {
                    st.playing = None;
                    let _ = games_played(&conn, None);
                }
                sink(Event::GameStopped { app_id });
            }
            _ = sleep_until(retry_at), if retry_at.is_some() && !connecting => {
                retry_at = None;
                connecting = true;
                let attempt = match st.link {
                    Link::Reconnecting { attempt, .. } => attempt,
                    _ => 1,
                };
                st.set_link(Link::Reconnecting { attempt, retry_in: 0 }, sink);
                let mut acc = store::find(&st.account).unwrap_or_else(|| account.clone());
                acc.refresh_token = st.refresh_token.clone();
                let tx = rc_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(reconnect(acc).await);
                });
            }
            Some(result) = rc_rx.recv() => {
                connecting = false;
                match result {
                    // "Go offline" was picked while this attempt ran.
                    Ok((c, _, _)) if !st.want_online => {
                        let _ = c.send(emsg::CLIENT_LOG_OFF, &CMsgClientLogOff {});
                    }
                    Ok((c, e, info)) => {
                        events = e;
                        quiet_tx = None;
                        hb = tokio::time::interval(Duration::from_secs(info.heartbeat.clamp(5, 60) as u64));
                        hb.tick().await;
                        st.reconnected(c, info, sink);
                        st.pump(sink, &dl_tx);
                    }
                    Err(e) if is_transient(&e) => {
                        tracing::info!("still can't reach Steam: {e}");
                        match st.link {
                            // A "Go online" from offline mode that didn't work: stay offline.
                            Link::Reconnecting { .. } if st.from_offline => {
                                st.want_online = false;
                                st.from_offline = false;
                                st.set_link(Link::Offline, sink);
                                sink(Event::Error(format!("Couldn't connect to Steam: {e}")));
                            }
                            Link::Reconnecting { attempt, .. } => {
                                let next = attempt + 1;
                                let wait = backoff(next);
                                st.set_link(Link::Reconnecting { attempt: next, retry_in: wait }, sink);
                                retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(wait as u64));
                            }
                            _ => {}
                        }
                    }
                    // The saved login doesn't work any more: sign in again.
                    Err(e) => return Ok(e.to_string()),
                }
            }
            Some((generation, dead)) = probe_rx.recv() => {
                if generation == st.generation {
                    st.probing = false;
                    if dead && st.link == Link::Online {
                        lost!("Steam stopped answering");
                    } else {
                        st.last_rx = std::time::Instant::now();
                    }
                }
            }
            _ = hb.tick() => {
                if st.link == Link::Online {
                    if conn.send(emsg::CLIENT_HEARTBEAT, &CMsgClientHeartBeat { send_reply: Some(false) }).is_err() {
                        lost!("the connection is closed");
                    } else if st.last_rx.elapsed() > Duration::from_secs(60) && !st.probing {
                        // Quiet for a minute: make sure the connection still works.
                        st.probing = true;
                        let (c, tx, generation) = (conn.clone(), probe_tx.clone(), st.generation);
                        tokio::spawn(async move {
                            let r = c.call::<_, CPlayerGetGameBadgeLevelsResponse>("Player.GetGameBadgeLevels#1", &CPlayerGetGameBadgeLevelsRequest { appid: Some(0) }).await;
                            let _ = tx.send((generation, matches!(r, Err(net::Error::Timeout | net::Error::Disconnected))));
                        });
                    }
                }
                if st.cache_dirty && st.cache_saved.elapsed() > Duration::from_secs(60) {
                    st.save_cache();
                }
            }
        }
        let _ = &quiet_tx;
    }
}

/// Wait until `at` (forever when `None`; the branch is disabled then anyway).
async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

fn set_persona(conn: &Connection, s: PersonaState) -> Result<(), Error> {
    conn.send(
        emsg::CLIENT_CHANGE_STATUS,
        &CMsgClientChangeStatus {
            persona_state: Some(s.as_u32()),
            persona_set_by_user: Some(true),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// `ClientGamesPlayedWithDataBlob`: the message Steam's client sends. With the
/// older `ClientGamesPlayed` (742) Steam marks you "In-Game" but shows no game.
const CLIENT_GAMES_PLAYED_WITH_DATA_BLOB: u32 = 5410;

fn games_played(conn: &Connection, app: Option<AppId>) -> Result<(), Error> {
    conn.send(
        CLIENT_GAMES_PLAYED_WITH_DATA_BLOB,
        &CMsgClientGamesPlayed {
            games_played: app
                .map(|id| c_msg_client_games_played::GamePlayed {
                    game_id: Some(id as u64),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            client_os_type: Some(OS_TYPE_WIN11),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn is_individual(id: SteamId) -> bool {
    (id >> 52) & 0xF == 1
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Online {
    /// Returns `Some` when the server ends the session.
    fn handle(
        &mut self,
        p: Packet,
        sink: &Sink,
        apps_tx: &mpsc::UnboundedSender<Vec<OwnedApp>>,
    ) -> Result<Option<Ended>, Error> {
        match p.emsg {
            emsg::CLIENT_LICENSE_LIST => {
                let list: CMsgClientLicenseList = p.body()?;
                let licenses: Vec<(u32, u64)> = list
                    .licenses
                    .iter()
                    .map(|l| (l.package_id(), l.access_token()))
                    .collect();
                sink(Event::Licenses { count: licenses.len() });
                let (conn, tx, sink) = (self.conn.clone(), apps_tx.clone(), sink.clone());
                tokio::spawn(async move {
                    match pics::resolve(&conn, &licenses).await {
                        Ok(apps) => {
                            let _ = tx.send(apps);
                        }
                        Err(e) => sink(Event::Error(format!("Could not load library: {e}"))),
                    }
                });
            }
            emsg::CLIENT_FRIENDS_LIST => {
                let list: CMsgClientFriendsList = p.body()?;
                if !list.bincremental() {
                    self.friends.clear();
                }
                let mut new_ids = Vec::new();
                for f in &list.friends {
                    let id = f.ulfriendid();
                    if !is_individual(id) {
                        continue;
                    }
                    let rel = Relationship::from_u32(f.efriendrelationship());
                    if rel == Relationship::None {
                        self.friends.remove(&id);
                    } else {
                        if !self.personas.contains_key(&id) {
                            new_ids.push(id);
                        }
                        self.friends.insert(id, rel);
                    }
                }
                sink(Event::Friends(self.friends.iter().map(|(k, v)| (*k, *v)).collect()));
                self.cache_dirty = true;
                if !new_ids.is_empty() {
                    self.conn.send(
                        emsg::CLIENT_REQUEST_FRIEND_DATA,
                        &CMsgClientRequestFriendData {
                            persona_state_requested: Some(PERSONA_FLAGS),
                            friends: new_ids,
                        },
                    )?;
                }
            }
            emsg::CLIENT_PERSONA_STATE => {
                let msg: CMsgClientPersonaState = p.body()?;
                let mut changed = Vec::new();
                let mut unknown_games = Vec::new();
                for f in msg.friends {
                    let id = f.friendid();
                    let entry = self
                        .personas
                        .entry(id)
                        .or_insert_with(|| Persona { steam_id: id, ..Default::default() });
                    if let Some(n) = f.player_name {
                        entry.name = n;
                    }
                    if let Some(s) = f.persona_state {
                        entry.state = PersonaState::from_u32(s);
                    }
                    if let Some(g) = f.game_played_app_id {
                        entry.game_app_id = (g != 0).then_some(g);
                    }
                    // Some games only fill the 64-bit game id (app id in the low 24 bits).
                    if entry.game_app_id.is_none() {
                        if let Some(gid) = f.gameid.filter(|g| *g != 0) {
                            let app = (gid & 0xFF_FFFF) as AppId;
                            entry.game_app_id = (app != 0 && (gid >> 56) == 0).then_some(app);
                        }
                    }
                    if f.persona_state.is_some() || f.game_played_app_id.is_some() {
                        let status = f.rich_presence.iter().find(|kv| kv.key() == "status").map(|kv| kv.value().trim().to_string());
                        entry.rich_status = status.filter(|s| !s.is_empty() && !s.starts_with('#'));
                    }
                    if let Some(app) = entry.game_app_id {
                        if !self.apps.contains_key(&app) && self.names_asked.insert(app) {
                            unknown_games.push(app);
                        }
                    }
                    if let Some(n) = f.game_name {
                        entry.game_name = (!n.is_empty()).then_some(n);
                    }
                    if let Some(h) = f.avatar_hash {
                        entry.avatar_hash = Some(hex(&h));
                    }
                    if let Some(t) = f.last_logoff {
                        entry.last_logoff = Some(t as u64);
                    }
                    if id == self.me {
                        let (name, avatar) = (entry.name.clone(), entry.avatar_hash.clone());
                        store::update(&self.account, |a| {
                            if !name.is_empty() {
                                a.persona_name = name;
                            }
                            a.avatar_hash = avatar.or(a.avatar_hash.take());
                        });
                    }
                    changed.push(entry.clone());
                }
                sink(Event::Personas(changed));
                self.cache_dirty = true;
                // Names for games friends play that aren't in our library.
                if !unknown_games.is_empty() {
                    let (conn, sink) = (self.conn.clone(), sink.clone());
                    tokio::spawn(async move {
                        if let Ok(names) = pics::names(&conn, &unknown_games).await {
                            sink(Event::AppNames(names));
                        }
                    });
                }
            }
            crate::ipc::EMSG_GAME_CONNECT_TOKENS => {
                let m: CMsgClientGameConnectTokens = p.body()?;
                self.auth.lock().unwrap().add_tokens(&m);
            }
            crate::ipc::EMSG_AUTH_LIST_ACK => {
                let m: CMsgClientAuthListAck = p.body()?;
                self.auth.lock().unwrap().on_ack(&m);
            }
            crate::ipc::EMSG_TICKET_AUTH_COMPLETE => {
                let m: CMsgClientTicketAuthComplete = p.body()?;
                self.auth.lock().unwrap().on_auth_complete(&m);
            }
            emsg::CLIENT_ACCOUNT_INFO => {
                let info: CMsgClientAccountInfo = p.body()?;
                let name = info.persona_name().to_string();
                store::update(&self.account, |a| a.persona_name = name.clone());
                self.country = info.ip_country().to_string();
                self.cache_dirty = true;
                sink(Event::AccountInfo { persona_name: name, country: info.ip_country().into() });
                // Ask for our own persona so the avatar shows up.
                self.conn.send(
                    emsg::CLIENT_REQUEST_FRIEND_DATA,
                    &CMsgClientRequestFriendData {
                        persona_state_requested: Some(PERSONA_FLAGS),
                        friends: vec![self.me],
                    },
                )?;
            }
            emsg::SERVICE_METHOD => {
                let name = p.header.target_job_name().to_string();
                if name.starts_with("SteamNotificationClient.NotificationsReceived") {
                    let n: proto::webui::CSteamNotificationNotificationsReceivedNotification = p.body()?;
                    let items: Vec<model::Notification> = n.notifications.iter().map(crate::notify::convert).collect();
                    if !items.is_empty() {
                        sink(Event::NotificationsArrived(items));
                    }
                    self.load_notifications(sink);
                } else if name.starts_with("FriendMessagesClient.IncomingMessage") {
                    let n: CFriendMessagesIncomingMessageNotification = p.body()?;
                    match n.chat_entry_type() {
                        1 => sink(Event::Message(model::ChatMessage {
                            friend: n.steamid_friend(),
                            from: if n.local_echo() { self.me } else { n.steamid_friend() },
                            text: if n.message_no_bbcode().is_empty() {
                                n.message().into()
                            } else {
                                n.message_no_bbcode().into()
                            },
                            time: n.rtime32_server_timestamp() as u64,
                        })),
                        2 => sink(Event::Typing { from: n.steamid_friend() }),
                        _ => {}
                    }
                } else {
                    sink(Event::Unhandled { emsg: p.emsg, name });
                }
            }
            emsg::CLIENT_EMAIL_ADDR_INFO => {
                let e: CMsgClientEmailAddrInfo = p.body()?;
                sink(Event::Email { address: e.email_address().to_string(), validated: e.email_is_validated() });
            }
            emsg::CLIENT_WALLET_INFO_UPDATE => {
                let w: CMsgClientWalletInfoUpdate = p.body()?;
                if w.has_wallet() {
                    let balance = w.balance64.unwrap_or(w.balance() as i64);
                    let pending = w.balance64_delayed.unwrap_or(w.balance_delayed() as i64);
                    self.wallet_currency = Some(w.currency());
                    sink(Event::Wallet { balance, currency: w.currency(), pending });
                }
            }
            emsg::CLIENT_LOGGED_OFF => {
                let off: CMsgClientLoggedOff = p.body()?;
                // Maintenance or a busy server: reconnect instead of signing out.
                if matches!(off.eresult(), 3 | 16 | eresult::SERVICE_UNAVAILABLE | eresult::TRY_ANOTHER_CM) {
                    return Ok(Some(Ended::Dropped));
                }
                return Ok(Some(Ended::Logout(eresult::describe(off.eresult(), ""))));
            }
            other => sink(Event::Unhandled { emsg: other, name: emsg::name(other) }),
        }
        Ok(None)
    }

    fn send_message(&self, to: SteamId, text: String, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            let r: Result<CFriendMessagesSendMessageResponse, _> = conn
                .call(
                    "FriendMessages.SendMessage#1",
                    &CFriendMessagesSendMessageRequest {
                        steamid: Some(to),
                        chat_entry_type: Some(1),
                        message: Some(text.clone()),
                        contains_bbcode: Some(false),
                        ..Default::default()
                    },
                )
                .await;
            match r {
                Ok(r) => sink(Event::Message(model::ChatMessage {
                    friend: to,
                    from: me,
                    text: if r.modified_message().is_empty() { text } else { r.modified_message().into() },
                    time: r.server_timestamp() as u64,
                })),
                Err(e) => sink(Event::Error(format!("Message not sent: {e}"))),
            }
        });
    }

    fn with_progress(&self, app_id: AppId, f: impl FnOnce(&depot::Progress)) {
        if let Some((id, p)) = &self.active {
            if *id == app_id {
                f(p);
            }
        }
    }

    /// Start the next queued download if nothing is running.
    fn pump(&mut self, sink: &Sink, done: &mpsc::UnboundedSender<(AppId, Result<String, String>)>) {
        // The queue is reported after the next game leaves it, so the one
        // that's downloading isn't also listed as "queued".
        if self.active.is_some() || self.link != Link::Online {
            sink(Event::DownloadQueue(self.queue.iter().map(|q| q.app_id).collect()));
            return;
        }
        let next = self.queue.pop_front();
        sink(Event::DownloadQueue(self.queue.iter().map(|q| q.app_id).collect()));
        let Some(req) = next else { return };
        let app_id = req.app_id;
        let Some(app) = self.apps.get(&app_id).cloned() else {
            let _ = done.send((app_id, Err("Your library is still loading. Try again in a moment.".into())));
            return;
        };
        self.active_req = Some(req.clone());
        let dest = req.library.join("steamapps").join("common").join(if app.install_dir.is_empty() { &app.name } else { &app.install_dir });
        let progress = std::sync::Arc::new(depot::Progress::default());
        progress.limit_kbps.store(self.limit_kbps, std::sync::atomic::Ordering::Relaxed);
        progress.concurrency.store(self.concurrency, std::sync::atomic::Ordering::Relaxed);
        self.active = Some((app_id, progress.clone()));
        let owned: std::collections::HashSet<AppId> = self.apps.keys().copied().collect();
        let (conn, cell, me, done, sink2) = (self.conn.clone(), self.download_cell.unwrap_or(self.cell_id), self.me, done.clone(), sink.clone());
        let host = self.download_host.clone();

        // Progress ticker: speed over a sliding window, ~2 updates per second.
        let (p2, sink3) = (progress.clone(), sink.clone());
        tokio::spawn(async move {
            let mut samples: std::collections::VecDeque<(std::time::Instant, u64)> = Default::default();
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let now = std::time::Instant::now();
                let bytes = p2.downloaded.load(std::sync::atomic::Ordering::Relaxed);
                samples.push_back((now, bytes));
                while samples.len() > 10 {
                    samples.pop_front();
                }
                let speed = match (samples.front(), samples.back()) {
                    (Some(a), Some(b)) if b.0 > a.0 => ((b.1 - a.1) as f64 / (b.0 - a.0).as_secs_f64()) as u64,
                    _ => 0,
                };
                let mut status = crate::download::status(app_id, &p2, speed);
                status.install_dir = Some(dest.clone());
                sink3(Event::Download(status));
                if p2.phase() == depot::Phase::Done || p2.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            let result = crate::download::run(conn, app, owned, req, cell, host, me, progress).await;
            let msg = result.map(|r| {
                format!(
                    "{} MB downloaded, {} MB reused, {} files unchanged",
                    r.downloaded_bytes / 1_048_576,
                    r.reused_bytes / 1_048_576,
                    r.skipped
                )
            });
            if let Err(e) = &msg {
                tracing::warn!("download {app_id} failed: {e}");
            }
            let _ = sink2;
            let _ = done.send((app_id, msg));
        });
    }

    /// Unfinished downloads (running first, then the queue), per account.
    fn downloads_path(&self) -> std::path::PathBuf {
        store::config_dir().join(format!("downloads-{}.json", self.me))
    }

    fn save_downloads(&self) {
        let list: Vec<&crate::download::InstallRequest> = self.active_req.iter().chain(self.queue.iter()).collect();
        let path = self.downloads_path();
        if list.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if let Ok(text) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::create_dir_all(store::config_dir());
            let _ = std::fs::write(&path, text);
        }
    }

    fn restore_downloads(&mut self, sink: &Sink, done: &mpsc::UnboundedSender<(AppId, Result<String, String>)>) {
        let Ok(text) = std::fs::read_to_string(self.downloads_path()) else { return };
        let Ok(list) = serde_json::from_str::<Vec<crate::download::InstallRequest>>(&text) else { return };
        for req in list {
            let busy = self.active.as_ref().is_some_and(|a| a.0 == req.app_id) || self.queue.iter().any(|q| q.app_id == req.app_id);
            if !busy && self.apps.contains_key(&req.app_id) {
                tracing::info!("resuming the download of {}", req.app_id);
                self.queue.push_back(req);
            }
        }
        self.pump(sink, done);
        self.save_downloads();
    }

    fn unsubscribe(&self, app_id: AppId, file_id: u64, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let r: Result<CPublishedFileUnsubscribeResponse, _> = conn
                .call(
                    "PublishedFile.Unsubscribe#1",
                    &CPublishedFileUnsubscribeRequest { publishedfileid: Some(file_id), list_type: Some(1), appid: Some(app_id as i32), notify_client: Some(true) },
                )
                .await;
            sink(Event::WorkshopUnsubscribed { file_id, result: r.map(|_| ()).map_err(|e| e.to_string()) });
        });
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn workshop_query(&self, request_id: u64, app_id: AppId, query_type: u32, page: u32, search: String, days: u32, tags: Vec<String>, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let req = CPublishedFileQueryFilesRequest {
                query_type: Some(if search.trim().is_empty() { query_type } else { 12 }),
                page: Some(page.max(1)),
                numperpage: Some(30),
                creator_appid: Some(app_id),
                appid: Some(app_id),
                search_text: Some(search).filter(|s| !s.trim().is_empty()),
                days: Some(days).filter(|d| *d > 0),
                return_previews: Some(true),
                return_short_description: Some(true),
                return_vote_data: Some(true),
                return_tags: Some(true),
                requiredtags: tags,
                match_all_tags: Some(true),
                ..Default::default()
            };
            match conn.call::<_, CPublishedFileQueryFilesResponse>("PublishedFile.QueryFiles#1", &req).await {
                Ok(r) => {
                    let items = r
                        .publishedfiledetails
                        .iter()
                        .map(|d| model::WorkshopEntry {
                            id: d.publishedfileid(),
                            title: d.title().to_string(),
                            preview: Some(d.preview_url().to_string()).filter(|p| !p.is_empty()),
                            description: d.short_description().to_string(),
                            size: d.file_size(),
                            subscriptions: d.subscriptions() as u64,
                            favorited: d.favorited() as u64,
                            updated: d.time_updated() as u64,
                            votes_up: d.vote_data.as_ref().map(|v| v.votes_up()).unwrap_or(0),
                            votes_down: d.vote_data.as_ref().map(|v| v.votes_down()).unwrap_or(0),
                            tags: d.tags.iter().filter(|t| !t.adminonly()).map(|t| t.tag().to_string()).filter(|t| !t.is_empty()).collect(),
                        })
                        .collect();
                    sink(Event::WorkshopResults { request_id, total: r.total() as u64, items });
                }
                Err(e) => sink(Event::StoreError(e.to_string())),
            }
        });
    }

    fn workshop_subscribe(&self, app_id: AppId, file_id: u64, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let r: Result<CPublishedFileSubscribeResponse, _> = conn
                .call(
                    "PublishedFile.Subscribe#1",
                    &CPublishedFileSubscribeRequest { publishedfileid: Some(file_id), list_type: Some(1), appid: Some(app_id as i32), notify_client: Some(true), include_dependencies: Some(true) },
                )
                .await;
            sink(Event::WorkshopSubscribed { file_id, result: r.map(|_| ()).map_err(|e| e.to_string()) });
        });
    }

    fn load_wishlist(&self, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            use proto::webui::*;
            match conn.call::<_, CWishlistGetWishlistResponse>("Wishlist.GetWishlist#1", &CWishlistGetWishlistRequest { steamid: Some(me) }).await {
                Ok(r) => sink(Event::Wishlist(r.items.iter().map(|i| (i.appid(), i.date_added() as u64)).collect())),
                Err(e) => sink(Event::StoreError(e.to_string())),
            }
        });
    }

    fn wishlist_change(&self, app_id: AppId, add: bool, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            use proto::webui::*;
            let r = if add {
                conn.call::<_, CWishlistAddToWishlistResponse>("Wishlist.AddToWishlist#1", &CWishlistAddToWishlistRequest { appid: Some(app_id), navdata: None }).await.map(|_| ())
            } else {
                conn.call::<_, CWishlistRemoveFromWishlistResponse>("Wishlist.RemoveFromWishlist#1", &CWishlistRemoveFromWishlistRequest { appid: Some(app_id) }).await.map(|_| ())
            };
            if let Err(e) = r {
                sink(Event::StoreError(e.to_string()));
            }
            if let Ok(r) = conn.call::<_, CWishlistGetWishlistResponse>("Wishlist.GetWishlist#1", &CWishlistGetWishlistRequest { steamid: Some(me) }).await {
                sink(Event::Wishlist(r.items.iter().map(|i| (i.appid(), i.date_added() as u64)).collect()));
            }
        });
    }

    /// The country the cart prices in: the wallet's (like Steam's client),
    /// else where Steam sees us. Steam needs it to add anything.
    fn cart_country(&self) -> Option<String> {
        model::store_country(self.wallet_currency, &self.country).map(|c| c.to_uppercase())
    }

    fn load_cart(&self, sink: &Sink) {
        let (conn, sink, country) = (self.conn.clone(), sink.clone(), self.cart_country());
        tokio::spawn(async move {
            use proto::webui::*;
            match conn.call::<_, CAccountCartGetCartResponse>("AccountCart.GetCart#1", &CAccountCartGetCartRequest { user_country: country }).await {
                Ok(r) => sink(Event::Cart(cart_lines(r.cart))),
                Err(e) => sink(Event::StoreError(e.to_string())),
            }
        });
    }

    fn cart_add(&self, package_id: u32, gift_to: Option<SteamId>, sink: &Sink) {
        let (conn, sink, country) = (self.conn.clone(), sink.clone(), self.cart_country());
        tokio::spawn(async move {
            use proto::webui::*;
            let item = CAccountCartAddItemsToCartRequestItemToAdd {
                packageid: Some(package_id),
                gift_info: gift_to.map(|id| CartGiftInfo { accountid_giftee: Some(model::account_id(id) as i32), ..Default::default() }),
                ..Default::default()
            };
            let req = CAccountCartAddItemsToCartRequest { user_country: country, items: vec![item], ..Default::default() };
            match conn.call::<_, CAccountCartAddItemsToCartResponse>("AccountCart.AddItemsToCart#1", &req).await {
                Ok(r) => {
                    let lines = cart_lines(r.cart);
                    // Steam can answer "OK" without adding anything: check.
                    let added = lines.iter().any(|l| l.package_id == package_id && l.gift_to == gift_to.map(|g| model::account_id(g) as u32));
                    if !added {
                        tracing::warn!("cart: package {package_id} not added (line items {:?})", r.line_item_ids);
                        sink(Event::StoreError("Steam didn't add it to your cart. It may not be sold in your region, or gifting it isn't allowed.".into()));
                    }
                    sink(Event::Cart(lines));
                }
                Err(e) => sink(Event::StoreError(format!("Couldn't add it to your cart: {e}"))),
            }
        });
    }

    fn cart_remove(&self, line_item_id: u64, sink: &Sink) {
        let (conn, sink, country) = (self.conn.clone(), sink.clone(), self.cart_country());
        tokio::spawn(async move {
            use proto::webui::*;
            match conn.call::<_, CAccountCartRemoveItemFromCartResponse>("AccountCart.RemoveItemFromCart#1", &CAccountCartRemoveItemFromCartRequest { line_item_id: Some(line_item_id), user_country: country }).await {
                Ok(r) => sink(Event::Cart(cart_lines(r.cart))),
                Err(e) => sink(Event::StoreError(e.to_string())),
            }
        });
    }

    fn activate_key(&self, key: String, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let req = CStoreRegisterCdKeyRequest {
                activation_code: Some(key.trim().to_string()),
                purchase_platform: None,
                is_request_from_client: Some(true),
            };
            let result = match conn.call_packet("Store.RegisterCDKey#1", &req).await {
                Ok(p) => {
                    let body: CStoreRegisterCdKeyResponse = p.body().unwrap_or_default();
                    let detail = body.purchase_result_details();
                    let items: Vec<String> = body
                        .purchase_receipt_info
                        .as_ref()
                        .map(|r| r.line_items.iter().map(|l| l.line_item_description().to_string()).collect())
                        .unwrap_or_default();
                    if p.eresult() == eresult::OK && detail == 0 {
                        Ok(items)
                    } else {
                        Err(key_error(detail, p.eresult()))
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            sink(Event::KeyActivated(result));
        });
    }

    fn load_profile(&self, steam_id: SteamId, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let mut info = model::ProfileInfo { steam_id, ..Default::default() };
            let items: Result<CPlayerGetProfileItemsEquippedResponse, _> = conn
                .call(
                    "Player.GetProfileItemsEquipped#1",
                    &CPlayerGetProfileItemsEquippedRequest { steamid: Some(steam_id), language: Some("english".into()) },
                )
                .await;
            if let Ok(items) = items {
                let url = |i: &Option<ProfileItem>| {
                    i.as_ref().map(|i| i.image_large()).filter(|s| !s.is_empty()).map(model::art::community_item)
                };
                info.background = url(&items.profile_background);
                info.avatar_frame = items
                    .avatar_frame
                    .as_ref()
                    .map(|i| i.image_small())
                    .filter(|s| !s.is_empty())
                    .map(model::art::community_item);
            }
            let reply = conn
                .request(emsg::CLIENT_FRIEND_PROFILE_INFO, &CMsgClientFriendProfileInfo { steamid_friend: Some(steam_id) })
                .await;
            if let Ok(r) = reply.and_then(|p| p.body::<CMsgClientFriendProfileInfoResponse>()) {
                info.real_name = r.real_name().into();
                info.summary = r.summary().into();
                info.location = [r.city_name(), r.state_name(), r.country_name()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                info.member_since = Some(r.time_created() as u64).filter(|t| *t > 0);
            }
            (info.visibility, info.vac_banned) = public_summary(steam_id).await;
            sink(Event::Profile(info));
        });
    }

    fn load_inventory(&self, steam_id: SteamId, app_id: AppId, context_id: u64, language: String, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let mut inv = model::Inventory { steam_id, app_id, context_id, ..Default::default() };
            let mut start = None;
            // Steam caps a page at 2000 items; stop at a sane total.
            while inv.items.len() < 20_000 {
                let req = CEconGetInventoryItemsWithDescriptionsRequest {
                    steamid: Some(steam_id),
                    appid: Some(app_id),
                    contextid: Some(context_id),
                    get_descriptions: Some(true),
                    language: Some(language.clone()),
                    start_assetid: start,
                    count: Some(2000),
                    ..Default::default()
                };
                match conn.call::<_, CEconGetInventoryItemsWithDescriptionsResponse>("Econ.GetInventoryItemsWithDescriptions#1", &req).await {
                    Ok(r) => {
                        inv.total = r.total_inventory_count();
                        let descs: HashMap<(u64, u64), &CEconItemDescription> = r.descriptions.iter().map(|d| ((d.classid(), d.instanceid()), d)).collect();
                        for a in &r.assets {
                            let Some(d) = descs.get(&(a.classid(), a.instanceid())) else { continue };
                            inv.items.push(econ_item(steam_id, a, d));
                        }
                        if r.more_items() && r.last_assetid() != 0 {
                            start = Some(r.last_assetid());
                        } else {
                            break;
                        }
                    }
                    Err(net::Error::EResult(code, _)) if code == eresult::ACCESS_DENIED => {
                        inv.error = Some("private".into());
                        break;
                    }
                    Err(e) => {
                        inv.error = Some(e.to_string());
                        break;
                    }
                }
            }
            if inv.total == 0 {
                inv.total = inv.items.len() as u32;
            }
            sink(Event::Inventory(inv));
        });
    }

    /// Run `f` with the signed-in web session (made on first use).
    fn with_web<F, Fut>(&self, sink: &Sink, f: F)
    where
        F: FnOnce(Result<std::sync::Arc<crate::market::Web>, String>, Sink) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (conn, sink, me, refresh, cache) = (self.conn.clone(), sink.clone(), self.me, self.refresh_token.clone(), self.web.clone());
        tokio::spawn(async move {
            let web = crate::market::web(&conn, me, &refresh, &cache).await;
            if let Err(e) = &web {
                tracing::warn!("web sign-in: {e}");
            }
            f(web, sink).await;
        });
    }

    fn load_inventory_apps(&self, steam_id: SteamId, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let (result, rules) = crate::market::inventory_apps(web.as_deref().ok(), steam_id).await;
            if let Some(rules) = rules {
                sink(Event::FeeRules(rules));
            }
            if let Err(e) = &result {
                tracing::info!("inventory list for {steam_id}: {e}");
            }
            sink(Event::InventoryApps { steam_id, result });
        });
    }

    fn market_price(&self, app_id: AppId, hash_name: String, currency: i32, country: String, full: bool, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = crate::market::price(web.as_deref().ok(), app_id, &hash_name, currency, &country, full).await;
            sink(Event::MarketPrice { app_id, hash_name, result });
        });
    }

    fn sell_item(&self, app_id: AppId, context_id: u64, asset_id: u64, amount: u64, receive: u64, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) => crate::market::sell(&w, app_id, context_id, asset_id, amount, receive).await,
                Err(e) => Err(e),
            };
            sink(Event::ItemListed { asset_id, result });
        });
    }

    fn my_listings(&self, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) => crate::market::my_listings(&w).await,
                Err(e) => Err(e),
            };
            sink(Event::MyListings(result));
        });
    }

    fn load_notifications(&self, sink: &Sink) {
        tokio::spawn(Online::fetch_notifications(self.conn.clone(), sink.clone()));
    }

    async fn fetch_notifications(conn: Connection, sink: Sink) {
        let req = proto::webui::CSteamNotificationGetSteamNotificationsRequest { include_hidden: Some(false), include_read: Some(true), ..Default::default() };
        match conn.call::<_, proto::webui::CSteamNotificationGetSteamNotificationsResponse>("SteamNotification.GetSteamNotifications#1", &req).await {
            Ok(r) => {
                let mut items: Vec<model::Notification> = r.notifications.iter().map(crate::notify::convert).collect();
                items.sort_by_key(|n| std::cmp::Reverse(n.time));
                sink(Event::Notifications(model::Notifications {
                    unread: r.unread_count.unwrap_or_else(|| items.iter().filter(|n| !n.read).count() as u32),
                    items,
                    pending_gifts: r.pending_gift_count(),
                    pending_friends: r.pending_friend_count(),
                    confirmations: r.confirmation_count().max(0) as u32,
                }));
            }
            Err(e) => tracing::warn!("notifications: {e}"),
        }
    }

    fn mark_read(&self, ids: Vec<u64>, sink: &Sink) {
        let req = proto::webui::CSteamNotificationMarkNotificationsReadNotification {
            mark_all_read: Some(ids.is_empty()),
            notification_ids: ids,
            ..Default::default()
        };
        if let Err(e) = self.conn.notify_service("SteamNotification.MarkNotificationsRead#1", &req) {
            tracing::warn!("mark read: {e}");
        }
        // Reload once Steam has applied it, so the counts are right.
        let (st, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            Online::fetch_notifications(st, sink).await;
        });
    }

    fn support_tickets(&self, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) => match crate::market::page(&w, "https://help.steampowered.com/en/wizard/HelpRequests").await {
                    Ok(body) => {
                        Ok(crate::support::parse_tickets(&body))
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            };
            sink(Event::SupportTickets(result));
        });
    }

    fn support_ticket(&self, id: String, url: String, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) if url.starts_with("https://help.steampowered.com/") => crate::market::page(&w, &url).await.map(|b| crate::support::parse_messages(&b)),
                Ok(_) => Err("not a Steam Support link".into()),
                Err(e) => Err(e),
            };
            sink(Event::SupportTicket { id, result });
        });
    }

    fn cancel_listing(&self, listing_id: u64, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) => crate::market::cancel_listing(&w, listing_id).await,
                Err(e) => Err(e),
            };
            sink(Event::ListingCancelled { listing_id, result });
        });
    }

    fn cancel_buy_order(&self, order_id: u64, sink: &Sink) {
        self.with_web(sink, move |web, sink| async move {
            let result = match web {
                Ok(w) => crate::market::cancel_buy_order(&w, order_id).await,
                Err(e) => Err(e),
            };
            sink(Event::BuyOrderCancelled { order_id, result });
        });
    }

    fn load_achievement_progress(&self, app_ids: Vec<AppId>, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            let req = CPlayerGetAchievementsProgressRequest { steamid: Some(me), language: Some("english".into()), appids: app_ids, ..Default::default() };
            match conn.call::<_, CPlayerGetAchievementsProgressResponse>("Player.GetAchievementsProgress#1", &req).await {
                Ok(r) => sink(Event::AchievementProgress(r.achievement_progress.iter().map(|a| (a.appid() as AppId, a.unlocked(), a.total())).collect())),
                Err(e) => tracing::debug!("achievement progress: {e}"),
            }
        });
    }

    fn load_achievements(&self, app_id: AppId, language: String, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            let schema = conn
                .call::<_, CPlayerGetGameAchievementsResponse>("Player.GetGameAchievements#1", &CPlayerGetGameAchievementsRequest { appid: Some(app_id), language: Some(language), hash_only: Some(false) })
                .await;
            let mine = conn.call::<_, CPlayerGetUserAchievementsResponse>("Player.GetUserAchievements#1", &CPlayerGetUserAchievementsRequest { steamid: Some(me), appid: Some(app_id) }).await;
            let result = match schema {
                Err(e) => Err(e.to_string()),
                Ok(schema) => {
                    let unlocked: HashMap<u32, u64> = mine.map(|m| m.achievements.iter().filter(|a| a.unlocked()).map(|a| (a.internal_key(), a.unlock_time() as u64)).collect()).unwrap_or_default();
                    let image = |file: &str| if file.is_empty() { String::new() } else { format!("https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/{app_id}/{file}") };
                    Ok(schema
                        .achievements
                        .iter()
                        .filter(|a| !a.archived())
                        .map(|a| model::Achievement {
                            name: a.localized_name().to_string(),
                            description: a.localized_desc().to_string(),
                            icon: image(a.icon()),
                            icon_gray: image(a.icon_gray()),
                            hidden: a.hidden(),
                            unlocked: unlocked.contains_key(&a.internal_key()),
                            unlock_time: unlocked.get(&a.internal_key()).copied().unwrap_or(0),
                            global_percent: a.player_percent_unlocked().parse().unwrap_or(0.0),
                        })
                        .collect())
                }
            };
            sink(Event::Achievements { app_id, result });
        });
    }

    fn load_player_games(&self, steam_id: SteamId, language: String, sink: &Sink) {
        let (conn, sink) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let req = CPlayerGetOwnedGamesRequest {
                steamid: Some(steam_id),
                include_appinfo: Some(true),
                include_played_free_games: Some(true),
                language: Some(language),
                ..Default::default()
            };
            let result = match conn.call::<_, CPlayerGetOwnedGamesResponse>("Player.GetOwnedGames#1", &req).await {
                // Steam answers a private profile with an empty reply.
                Ok(r) if r.game_count.is_none() && r.games.is_empty() => Err("private".to_string()),
                Ok(r) => Ok(r
                    .games
                    .iter()
                    .map(|g| model::PlayerGame {
                        app_id: g.appid() as AppId,
                        name: g.name().to_string(),
                        playtime_minutes: g.playtime_forever().max(0) as u64,
                        playtime_2weeks: g.playtime_2weeks().max(0) as u64,
                        last_played: Some(g.rtime_last_played() as u64).filter(|t| *t > 0),
                        icon: Some(g.img_icon_url().to_string()).filter(|s| !s.is_empty()),
                    })
                    .collect()),
                Err(net::Error::EResult(code, _)) if code == eresult::ACCESS_DENIED => Err("private".to_string()),
                Err(e) => Err(e.to_string()),
            };
            sink(Event::PlayerGames { steam_id, result });
        });
    }

    fn load_history(&self, friend: SteamId, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            let r: Result<CFriendMessagesGetRecentMessagesResponse, _> = conn
                .call(
                    "FriendMessages.GetRecentMessages#1",
                    &CFriendMessagesGetRecentMessagesRequest {
                        steamid1: Some(me),
                        steamid2: Some(friend),
                        count: Some(50),
                        most_recent_conversation: Some(false),
                        bbcode_format: Some(false),
                        ..Default::default()
                    },
                )
                .await;
            let base = me & 0xFFFF_FFFF_0000_0000;
            match r {
                Ok(r) => {
                    let mut messages: Vec<model::ChatMessage> = r
                        .messages
                        .iter()
                        .map(|m| model::ChatMessage {
                            friend,
                            from: base | m.accountid() as u64,
                            text: m.message().to_string(),
                            time: m.timestamp() as u64,
                        })
                        .collect();
                    messages.sort_by_key(|m| m.time);
                    sink(Event::History { friend, messages });
                }
                Err(e) => tracing::warn!("chat history: {e}"),
            }
        });
    }

    /// What Steam Cloud needs for this game, if it uses the cloud and the
    /// player hasn't turned it off.
    fn cloud_target(&self, app_id: AppId, install_dir: &std::path::Path) -> Option<crate::cloud::Target> {
        if self.cloud_off.contains(&app_id) {
            return None;
        }
        self.cloud_target_any(app_id, install_dir)
    }

    fn cloud_target_any(&self, app_id: AppId, install_dir: &std::path::Path) -> Option<crate::cloud::Target> {
        let config = self.apps.get(&app_id)?.cloud.clone()?;
        Some(crate::cloud::Target { app_id, steam_id: self.me, install_dir: install_dir.to_path_buf(), steam_root: self.steam_root.clone(), config: Some(config), cell_id: self.cell_id })
    }

    fn launch(
        &mut self,
        app_id: AppId,
        dir: &std::path::Path,
        args: &str,
        sink: &Sink,
        exit_tx: &mpsc::UnboundedSender<AppId>,
    ) -> Result<(), Error> {
        let Some(app) = self.apps.get(&app_id) else {
            sink(Event::Error("Library is still loading, try again in a moment".into()));
            return Ok(());
        };
        if self.running.contains_key(&app_id) {
            sink(Event::Error(format!("{} is already running", app.name)));
            return Ok(());
        }
        let mode = launch::mode(app, dir);
        tracing::info!("{} starts as {mode:?}", app.name);
        sink(Event::LaunchMode { app_id, mode: mode.clone() });
        match mode {
            crate::LaunchMode::OfficialSteam { .. } => {
                // The official client runs it; we only follow along. No
                // GamesPlayed from us (Steam sends its own) and no cloud sync.
                self.running.insert(app_id, 0);
                self.official.insert(app_id, dir.to_path_buf());
                let (tx, sink, args, me, offline) = (exit_tx.clone(), sink.clone(), args.to_string(), self.me, self.link != Link::Online);
                tokio::task::spawn_blocking(move || {
                    let started = || sink(Event::GameStarted { app_id });
                    if let Err(e) = launch::run_through_steam(app_id, &args, me, offline, started) {
                        sink(Event::Error(e));
                    }
                    let _ = tx.send(app_id);
                });
                return Ok(());
            }
            crate::LaunchMode::SteamInRust => {
                let token = self.ipc.register(app_id);
                let dll = launch::steamclient_dll();
                let account_id = model::account_id(self.me);
                let swapped = dll.as_deref().ok_or_else(|| "steamclient DLL missing".to_string()).and_then(|d| launch::swap_begin(&token, d, account_id));
                let started = swapped.and_then(|_| launch::spawn_with_steamclient(app, dir, args, &self.ipc.env_value(&token), &self.account));
                match started {
                    Ok(child) => {
                        self.launch_tokens.insert(app_id, token.clone());
                        // A game that never loads the DLL mustn't keep it in the registry.
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(300)).await;
                            launch::swap_loaded(&token);
                        });
                        self.track(app_id, dir, child, sink, exit_tx)?;
                    }
                    Err(e) => {
                        launch::swap_exited(&token);
                        self.ipc.forget(&token);
                        sink(Event::Error(e));
                    }
                }
            }
            crate::LaunchMode::NoSteam { .. } => match launch::spawn(app, dir, args) {
                Ok(child) => self.track(app_id, dir, child, sink, exit_tx)?,
                Err(e) => sink(Event::Error(e)),
            },
        }
        Ok(())
    }

    /// A game we started ourselves: we're the one "playing" it.
    fn track(&mut self, app_id: AppId, dir: &std::path::Path, child: std::process::Child, sink: &Sink, exit_tx: &mpsc::UnboundedSender<AppId>) -> Result<(), Error> {
        self.running.insert(app_id, child.id());
        self.launched_dirs.insert(app_id, dir.to_path_buf());
        self.playing = Some(app_id);
        if self.link == Link::Online {
            let _ = games_played(&self.conn, Some(app_id));
        } else {
            // Steam won't see this; keep the time here.
            self.unconnected_play.insert(app_id, (unix_now(), self.link == Link::Offline));
        }
        sink(Event::GameStarted { app_id });
        let tx = exit_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(launch::wait(app_id, child));
        });
        Ok(())
    }

    /// A request from a game running with our steamclient DLL.
    fn ipc_call(&mut self, call: crate::ipc::Call, _sink: &Sink) {
        use crate::ipc::protocol::{Reply, Request};
        let crate::ipc::Call { app_id, token, pid, req, reply } = call;
        let (conn, me) = (self.conn.clone(), self.me);
        match req {
            Request::Hello if self.link != Link::Online => {
                tracing::info!("game {app_id} (pid {pid}) connected to SteamInRust's steamclient (offline)");
                launch::swap_loaded(&token);
                let mut session = self.ipc_session(app_id);
                session.stats = offline::offline_view(&offline::load_stats(me, app_id));
                let _ = reply.send(Reply::Hello(Box::new(session)));
            }
            Request::UserStats { steam_id } if self.link != Link::Online => {
                let result = if steam_id == me { offline::offline_view(&offline::load_stats(me, app_id)).ok_or_else(|| "offline".to_string()) } else { Err("offline".into()) };
                let _ = reply.send(Reply::Stats { result });
            }
            Request::StoreStats { stats, crc, reset } if self.link != Link::Online => {
                // Like Steam offline: kept here, sent once back online.
                offline::store_offline(me, app_id, &stats, reset);
                tracing::info!("game {app_id} stored {} stat(s) while offline", stats.len());
                let _ = reply.send(Reply::Stored { eresult: eresult::OK, crc });
            }
            Request::Hello => {
                tracing::info!("game {app_id} (pid {pid}) connected to SteamInRust's steamclient");
                launch::swap_loaded(&token);
                let session = self.ipc_session(app_id);
                let language = session.game_language.clone();
                tokio::spawn(async move {
                    let mut session = session;
                    let badges = async {
                        conn.call::<_, CPlayerGetGameBadgeLevelsResponse>("Player.GetGameBadgeLevels#1", &CPlayerGetGameBadgeLevelsRequest { appid: Some(0) })
                            .await
                            .ok()
                            .map(|r| r.player_level())
                    };
                    let (stats, level) = tokio::join!(crate::ipc::user_stats(&conn, app_id, me, &language), badges);
                    let mut cached = offline::load_stats(me, app_id);
                    match stats {
                        // Changes from an offline session not sent yet still count.
                        Ok(s) if !cached.pending.is_empty() || cached.reset => {
                            cached.stats = Some(s);
                            session.stats = offline::offline_view(&cached);
                        }
                        Ok(s) => {
                            offline::remember_stats(me, app_id, &s);
                            session.stats = Some(s);
                        }
                        Err(e) => {
                            tracing::info!("no stats for {app_id}: {e}");
                            session.stats = offline::offline_view(&cached);
                        }
                    }
                    session.level = level.unwrap_or(0);
                    let _ = reply.send(Reply::Hello(Box::new(session)));
                });
            }
            Request::UserStats { steam_id } => {
                let language = self.ipc_session(app_id).game_language;
                tokio::spawn(async move {
                    let result = crate::ipc::user_stats(&conn, app_id, steam_id, &language).await;
                    let _ = reply.send(Reply::Stats { result });
                });
            }
            Request::StoreStats { stats, crc, reset } => {
                tokio::spawn(async move {
                    let (mut eresult, crc) = crate::ipc::store_stats(&conn, app_id, me, &stats, crc, reset).await;
                    tracing::info!("game {app_id} stored {} stat(s): result {eresult}", stats.len());
                    if eresult == 3 {
                        // The connection is gone: keep them for later, like Steam.
                        offline::store_offline(me, app_id, &stats, reset);
                        eresult = net::eresult::OK;
                    }
                    let _ = reply.send(Reply::Stored { eresult, crc });
                });
            }
            Request::RichPresence { pairs } => {
                crate::ipc::rich_presence(&conn, &pairs);
                let _ = reply.send(Reply::Ok);
            }
            Request::CurrentPlayers => {
                tokio::spawn(async move {
                    let count = crate::ipc::current_players(&conn, app_id).await;
                    let _ = reply.send(Reply::Players { count });
                });
            }
            Request::AuthTicket { identity } => {
                let auth = self.auth.clone();
                tokio::spawn(async move {
                    if !identity.is_empty() {
                        tracing::info!("game {app_id} wants a web API ticket for {identity}");
                    }
                    let r = match crate::ipc::auth_ticket(&conn, &auth, app_id, pid).await {
                        Ok((ticket, crc, seq)) => Reply::Ticket { ticket, crc, seq },
                        Err(message) => Reply::Err { message },
                    };
                    let _ = reply.send(r);
                });
            }
            Request::AuthTicketWait { seq } => {
                let auth = self.auth.clone();
                tokio::spawn(async move {
                    let ok = crate::ipc::wait_ack(&auth, seq).await;
                    let _ = reply.send(if ok { Reply::Ok } else { Reply::Err { message: "Steam didn't confirm the ticket".into() } });
                });
            }
            Request::CancelTicket { crc } => {
                crate::ipc::cancel_ticket(&conn, &self.auth, crc);
                let _ = reply.send(Reply::Ok);
            }
            Request::BeginAuth { steam_id, ticket } => {
                let auth = self.auth.clone();
                tokio::spawn(async move {
                    let (response, owner) = crate::ipc::begin_auth(&conn, &auth, app_id, pid, steam_id, &ticket).await;
                    let _ = reply.send(Reply::AuthResult { response, owner });
                });
            }
            Request::EndAuth { steam_id } => {
                crate::ipc::end_auth(&conn, &self.auth, app_id, steam_id);
                let _ = reply.send(Reply::Ok);
            }
            Request::EncryptedTicket { data } => {
                tokio::spawn(async move {
                    let r = match crate::ipc::encrypted_ticket(&conn, app_id, &data).await {
                        Ok(data) => Reply::Bytes { data },
                        Err(message) => Reply::Err { message },
                    };
                    let _ = reply.send(r);
                });
            }
        }
    }

    /// What a game learns about the player at start-up.
    fn ipc_session(&self, app_id: AppId) -> crate::ipc::protocol::Session {
        use crate::ipc::protocol::{Friend, Session};
        let install_dir = self.launched_dirs.get(&app_id).cloned().unwrap_or_default();
        let (language, beta, build_id) = crate::ipc::manifest_info(app_id, &install_dir);
        let ui_language = crate::ipc::steam_ui_language().unwrap_or_else(|| "english".into());
        let me = self.personas.get(&self.me);
        Session {
            app_id,
            steam_id: self.me,
            account: self.account.clone(),
            persona_name: me
                .map(|p| p.name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| store::find(&self.account).map(|a| a.persona_name).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| self.account.clone()),
            country: self.country.clone(),
            game_language: language.unwrap_or_else(|| "english".into()),
            ui_language,
            level: 0,
            owned: self.apps.keys().copied().collect(),
            dlc: self.apps.values().filter(|a| a.parent == Some(app_id)).map(|a| (a.app_id, a.name.clone())).collect(),
            friends: self
                .friends
                .iter()
                .map(|(id, rel)| {
                    let p = self.personas.get(id);
                    Friend {
                        steam_id: *id,
                        name: p.map(|p| p.name.clone()).unwrap_or_default(),
                        state: p.map(|p| p.state.as_u32()).unwrap_or(0),
                        relationship: relationship_code(*rel),
                        game: p.and_then(|p| p.game_app_id).unwrap_or(0),
                    }
                })
                .collect(),
            install_dir: install_dir.to_string_lossy().into_owned(),
            steam_root: self.steam_root.clone().unwrap_or_else(|| store::config_dir().join("steam")).to_string_lossy().into_owned(),
            build_id,
            beta,
            stats: None,
            offline: self.link != Link::Online,
        }
    }
}

/// Connection changes and the offline cache.
impl Online {
    fn set_link(&mut self, link: Link, sink: &Sink) {
        if self.link != link {
            self.link = link;
            sink(Event::Link(link));
        }
    }

    /// Stop using the connection: it dropped, or the player chose "Go offline".
    /// Games keep running; their time from now on is kept here.
    fn drop_connection(&mut self, sink: &Sink, offline_mode: bool) -> (mpsc::UnboundedSender<Packet>, net::connection::Events) {
        if self.cache_dirty {
            self.save_cache();
        }
        self.conn = Connection::closed();
        self.generation += 1;
        self.probing = false;
        self.from_offline = false;
        let now = unix_now();
        for app in self.running.keys() {
            if !self.official.contains_key(app) {
                self.unconnected_play.entry(*app).or_insert((now, offline_mode));
            }
        }
        // Downloads wait for the connection (Steam pauses them too).
        if let Some((_, p)) = &self.active {
            if !p.paused.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.auto_paused = true;
            }
        }
        self.set_link(if offline_mode { Link::Offline } else { Link::Reconnecting { attempt: 1, retry_in: backoff(1) } }, sink);
        quiet_events()
    }

    /// Signed in again on a new connection: same session for the UI.
    fn reconnected(&mut self, conn: Connection, info: Logon, sink: &Sink) {
        tracing::info!("reconnected to Steam");
        self.conn = conn;
        self.generation += 1;
        self.last_rx = std::time::Instant::now();
        self.probing = false;
        self.from_offline = false;
        self.me = info.steam_id;
        self.cell_id = info.cell_id;
        self.auth = crate::ipc::Auth::new(info.public_ip);
        self.web = Default::default();
        let now = unix_now();
        for (app_id, (start, offline_mode)) in std::mem::take(&mut self.unconnected_play) {
            offline::record_play(self.me, offline::PlaySession { app_id, start, seconds: now.saturating_sub(start), offline: offline_mode, reported: false });
        }
        self.set_link(Link::Online, sink);
        sink(Event::LoggedIn { steam_id: self.me, account: self.account.clone() });
        self.connected(sink);
        if self.auto_paused {
            self.auto_paused = false;
            if let Some((_, p)) = &self.active {
                p.paused.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.flush_offline(sink);
        self.flush_cloud(sink);
        self.send_offline_lists(sink);
    }

    /// What a fresh connection needs: our status, the game we're in, level
    /// and notifications. Steam sends licenses, friends and personas itself.
    fn connected(&self, sink: &Sink) {
        let _ = set_persona(&self.conn, self.persona_state);
        if let Some(app) = self.playing {
            let _ = games_played(&self.conn, Some(app));
        }
        let (conn, sink2) = (self.conn.clone(), sink.clone());
        tokio::spawn(async move {
            let r: Result<CPlayerGetGameBadgeLevelsResponse, _> = conn.call("Player.GetGameBadgeLevels#1", &CPlayerGetGameBadgeLevelsRequest { appid: Some(0) }).await;
            if let Ok(r) = r {
                sink2(Event::Level(r.player_level()));
            }
        });
        self.load_notifications(sink);
    }

    /// Offline mode: the library and friends as they were at the last sign-in.
    fn start_offline(&mut self, cache: offline::AccountCache, sink: &Sink) {
        tracing::info!("starting {} in offline mode ({} apps from {})", self.account, cache.apps.len(), cache.saved);
        self.apps = cache.apps.iter().map(|a| (a.app_id, a.clone())).collect();
        self.friends = cache.friends.iter().copied().collect();
        self.country = cache.country.clone();
        for mut p in cache.personas {
            // Nobody's status is known offline.
            p.state = PersonaState::Offline;
            p.game_app_id = None;
            p.game_name = None;
            p.rich_status = None;
            self.personas.insert(p.steam_id, p);
        }
        let saved = store::find(&self.account);
        let name = Some(cache.persona_name.clone()).filter(|n| !n.is_empty()).or_else(|| saved.as_ref().map(|a| a.persona_name.clone())).unwrap_or_else(|| self.account.clone());
        let avatar = cache.avatar_hash.clone().or_else(|| saved.and_then(|a| a.avatar_hash));
        self.personas.insert(self.me, Persona { steam_id: self.me, name: name.clone(), state: PersonaState::Offline, avatar_hash: avatar, ..Default::default() });
        sink(Event::Link(Link::Offline));
        sink(Event::LoggedIn { steam_id: self.me, account: self.account.clone() });
        sink(Event::AccountInfo { persona_name: name, country: cache.country });
        sink(Event::Apps(cache.apps));
        sink(Event::Friends(self.friends.iter().map(|(k, v)| (*k, *v)).collect()));
        sink(Event::Personas(self.personas.values().cloned().collect()));
    }

    /// Offline play time and waiting cloud uploads, for the UI.
    fn send_offline_lists(&self, sink: &Sink) {
        sink(Event::OfflinePlaytime(offline::playtime_totals(&offline::play_sessions(self.me))));
        sink(Event::CloudPending(offline::cloud_pending(self.me).into_iter().map(|p| p.0).collect()));
    }

    /// Write the offline cache (only from live data).
    fn save_cache(&mut self) {
        if self.link != Link::Online || self.apps.is_empty() {
            return;
        }
        self.cache_dirty = false;
        self.cache_saved = std::time::Instant::now();
        let saved = store::find(&self.account);
        let me = self.personas.get(&self.me);
        let cache = offline::AccountCache {
            steam_id: self.me,
            account: self.account.clone(),
            persona_name: me.map(|p| p.name.clone()).filter(|n| !n.is_empty()).or_else(|| saved.as_ref().map(|a| a.persona_name.clone())).unwrap_or_default(),
            avatar_hash: me.and_then(|p| p.avatar_hash.clone()).or_else(|| saved.and_then(|a| a.avatar_hash)),
            country: self.country.clone(),
            apps: self.apps.values().cloned().collect(),
            friends: self.friends.iter().map(|(k, v)| (*k, *v)).collect(),
            personas: self
                .friends
                .keys()
                .filter_map(|id| self.personas.get(id))
                .map(|p| Persona { steam_id: p.steam_id, name: p.name.clone(), avatar_hash: p.avatar_hash.clone(), last_logoff: p.last_logoff, ..Default::default() })
                .collect(),
            saved: unix_now(),
        };
        tokio::task::spawn_blocking(move || offline::save_account(&cache));
    }

    /// Send a game's changed saves up; if that fails, try again once online.
    fn upload_saves(&self, app_id: AppId, target: crate::cloud::Target, sink: &Sink) {
        if !self.cloud_flushing.lock().unwrap().insert(app_id) {
            return;
        }
        sink(Event::CloudSyncing { app_id });
        let (conn, sink, me, flushing) = (self.conn.clone(), sink.clone(), self.me, self.cloud_flushing.clone());
        tokio::spawn(async move {
            let dir = target.install_dir.clone();
            let report = crate::cloud::sync(&conn, &target, crate::cloud::Mode::Upload, &[], &[]).await;
            flushing.lock().unwrap().remove(&app_id);
            if report.errors.is_empty() && report.conflicts.is_empty() {
                offline::remove_cloud_pending(me, app_id);
            } else {
                // Not reachable, or the player must pick a side first.
                offline::add_cloud_pending(me, app_id, &dir);
            }
            sink(Event::CloudPending(offline::cloud_pending(me).into_iter().map(|p| p.0).collect()));
            sink(Event::CloudReport { report, held_launch: None });
        });
    }

    /// Upload the saves of games played while offline.
    fn flush_cloud(&self, sink: &Sink) {
        // Needs the cloud settings of the library (and where userdata is).
        if self.link != Link::Online || self.apps.is_empty() || self.steam_root.is_none() {
            return;
        }
        for (app_id, dir) in offline::cloud_pending(self.me) {
            if self.running.contains_key(&app_id) {
                continue; // goes up when it closes
            }
            match self.cloud_target(app_id, &dir) {
                Some(t) => {
                    tracing::info!("uploading saves of {app_id} changed while offline");
                    self.upload_saves(app_id, t, sink);
                }
                // Cloud turned off for it, or no longer owned.
                None => offline::remove_cloud_pending(self.me, app_id),
            }
        }
    }

    /// Back online: send play time and game stats recorded while offline.
    fn flush_offline(&self, sink: &Sink) {
        let (conn, sink, me) = (self.conn.clone(), sink.clone(), self.me);
        tokio::spawn(async move {
            let unsent: Vec<offline::PlaySession> = offline::play_sessions(me).into_iter().filter(|s| !s.reported).collect();
            if !unsent.is_empty() && offline::SEND_TO_STEAM {
                // What Steam's own client sends for time played while disconnected.
                let req = CPlayerRecordDisconnectedPlaytimeRequest {
                    play_sessions: unsent
                        .iter()
                        .map(|s| c_player_record_disconnected_playtime_request::PlayHistory {
                            appid: Some(s.app_id),
                            session_time_start: Some(s.start as u32),
                            seconds: Some(s.seconds.min(u32::MAX as u64) as u32),
                            offline: Some(s.offline),
                            owner: Some(model::account_id(me)),
                        })
                        .collect(),
                };
                match conn.call::<_, CPlayerRecordDisconnectedPlaytimeResponse>("Player.RecordDisconnectedPlaytime#1", &req).await {
                    Ok(_) => {
                        tracing::info!("sent {} offline play session(s) to Steam", unsent.len());
                        offline::mark_reported(me, &unsent);
                    }
                    Err(e) => tracing::warn!("offline play time not sent: {e}"),
                }
                sink(Event::OfflinePlaytime(offline::playtime_totals(&offline::play_sessions(me))));
            }
            if offline::SEND_TO_STEAM {
                for app_id in offline::stats_pending(me) {
                    send_offline_stats(&conn, me, app_id).await;
                }
            }
        });
    }
}

/// Send stats and achievements a game stored while offline.
async fn send_offline_stats(conn: &Connection, me: SteamId, app_id: AppId) {
    let pending = offline::load_stats(me, app_id);
    if pending.pending.is_empty() && !pending.reset {
        return;
    }
    let current = match crate::ipc::user_stats(conn, app_id, me, "english").await {
        Ok(s) => s,
        Err(e) => return tracing::warn!("offline stats of {app_id} not sent: {e}"),
    };
    let values: Vec<(u32, u32)> = pending
        .pending
        .iter()
        .map(|(&id, &v)| match current.stats.iter().find(|s| s.id == id) {
            // Achievements unlocked elsewhere in the meantime stay unlocked.
            Some(s) if s.kind == 4 && !pending.reset => (id, v | s.value),
            _ => (id, v),
        })
        .collect();
    let (result, _) = crate::ipc::store_stats(conn, app_id, me, &values, current.crc, pending.reset).await;
    if result == eresult::OK {
        tracing::info!("sent {} stat(s) of {app_id} stored while offline", values.len());
        offline::stats_sent(me, app_id);
        if let Ok(fresh) = crate::ipc::user_stats(conn, app_id, me, "english").await {
            offline::remember_stats(me, app_id, &fresh);
        }
    } else {
        tracing::warn!("Steam didn't take the offline stats of {app_id}: result {result}");
    }
}


/// EFriendRelationship number.
fn relationship_code(r: Relationship) -> u32 {
    match r {
        Relationship::None => 0,
        Relationship::Blocked => 1,
        Relationship::RequestRecipient => 2,
        Relationship::Friend => 3,
        Relationship::RequestInitiator => 4,
        Relationship::Ignored => 5,
        Relationship::Other(v) => v,
    }
}


/// Steam's purchase result details for key activation (EPurchaseResultDetail).
fn key_error(detail: i32, result: i32) -> String {
    match detail {
        9 => "You already own this product",
        13 => "This product isn't available in your region",
        14 => "That key isn't valid",
        15 => "That key has already been used",
        24 => "This needs a game you don't own (for example, the base game)",
        53 => "Too many attempts. Try again in an hour",
        54 => "You own something that prevents this activation",
        _ => return net::eresult::describe(result, &format!("purchase detail {detail}")),
    }
    .into()
}

fn cart_lines(cart: Option<proto::webui::AccountCartContents>) -> Vec<model::CartLine> {
    cart.map(|c| {
        c.line_items
            .iter()
            .map(|l| model::CartLine {
                line_item_id: l.line_item_id(),
                package_id: l.packageid(),
                price: l.price_when_added.as_ref().map(|p| p.formatted_amount().to_string()).unwrap_or_default(),
                gift_to: l.gift_info.as_ref().map(|g| g.accountid_giftee() as u32).filter(|a| *a != 0),
            })
            .collect()
    })
    .unwrap_or_default()
}

fn econ_item(owner: SteamId, a: &CEconAsset, d: &CEconItemDescription) -> model::InventoryItem {
    let color = |c: &str| Some(c.trim_start_matches('#').to_string()).filter(|c| !c.is_empty());
    model::InventoryItem {
        asset_id: a.assetid(),
        class_id: a.classid(),
        amount: a.amount().max(1) as u64,
        name: if d.name().is_empty() { d.market_name().to_string() } else { d.name().to_string() },
        kind: d.r#type().to_string(),
        name_color: color(d.name_color()),
        background: color(d.background_color()),
        icon: d.icon_url().to_string(),
        icon_large: Some(d.icon_url_large().to_string()).filter(|s| !s.is_empty()),
        tradable: d.tradable(),
        marketable: d.marketable(),
        market_hash_name: d.market_hash_name().to_string(),
        description: d.descriptions.iter().filter(|l| !l.value().trim().is_empty()).map(|l| (l.value().to_string(), color(l.color()))).collect(),
        tags: d
            .tags
            .iter()
            .map(|t| (t.localized_category_name().to_string(), t.localized_tag_name().to_string(), color(t.color())))
            .collect(),
        actions: d
            .actions
            .iter()
            .map(|x| (x.name().to_string(), x.link().replace("%owner_steamid%", &owner.to_string()).replace("%assetid%", &a.assetid().to_string())))
            .collect(),
        context_id: a.contextid(),
        owner_notes: d.owner_descriptions.iter().map(|l| l.value().trim().to_string()).filter(|v| !v.is_empty()).collect(),
        trade_hold_days: d.market_tradable_restriction().max(0) as u32,
        market_hold_days: d.market_marketable_restriction().max(0) as u32,
        commodity: d.commodity(),
        market_fee: d.market_fee().parse().ok(),
    }
}

/// A profile's community visibility, from its public XML summary.
/// Visibility and VAC status from the profile's public XML summary.
async fn public_summary(steam_id: SteamId) -> (model::Visibility, bool) {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let http = HTTP.get_or_init(|| reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default());
    let url = format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1");
    let Ok(body) = async { http.get(url).send().await?.text().await }.await else { return (model::Visibility::Unknown, false) };
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let start = body.find(&open)? + open.len();
        let end = body[start..].find('<')? + start;
        Some(body[start..end].trim().to_string())
    };
    let vis = match tag("visibilityState").as_deref() {
        Some("3") => model::Visibility::Public,
        Some("2") => model::Visibility::FriendsOnly,
        Some("1") => model::Visibility::Private,
        _ => model::Visibility::Unknown,
    };
    (vis, tag("vacBanned").as_deref() == Some("1"))
}


/// This run's session id: stable while the app runs (so reconnecting replaces
/// our own old session), different from any other run or tool signed in to
/// the same account.
fn logon_id() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let mixed = (nanos as u64) ^ ((std::process::id() as u64) << 32);
        let mixed = mixed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        ((mixed >> 32) as u32).max(1)
    })
}

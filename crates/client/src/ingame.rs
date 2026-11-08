//! The app's end of the in-game overlay pipe (see
//! `crates/steamclient/src/overlay`).
//!
//! A game whose frames go through our steamclient DLL's D3D11 hook connects
//! to `<ipc pipe>-overlay` with its launch token. Each accepted connection
//! becomes a [`Conn`] the UI picks up with [`take_new`]: it runs the overlay
//! UI for that game and sends frames back with [`Conn::send`].

#[path = "../../steamclient/src/overlay/wire.rs"]
pub mod wire;

use model::AppId;
use std::sync::{Arc, Mutex, OnceLock};
use wire::{FromGame, ToGame};

/// One game process drawing the overlay itself.
pub struct Conn {
    pub app_id: AppId,
    pub pid: u32,
    /// Back buffer size from the game's hello.
    pub size: (u32, u32),
    pub from_game: std::sync::mpsc::Receiver<FromGame>,
    to_game: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl Conn {
    /// Queue a message; false once the game is gone.
    pub fn send(&self, m: &ToGame) -> bool {
        self.to_game.send(m.encode()).is_ok()
    }

    pub fn is_closed(&self) -> bool {
        self.to_game.is_closed()
    }
}

static NEW: Mutex<Vec<Conn>> = Mutex::new(Vec::new());
type Waker = Arc<dyn Fn() + Send + Sync>;
static WAKER: OnceLock<Waker> = OnceLock::new();

/// Called whenever a game connects or sends input (the UI repaints).
pub fn set_waker(f: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Arc::new(f));
}

fn wake() {
    if let Some(w) = WAKER.get() {
        w();
    }
}

/// Connections made since the last call.
pub fn take_new() -> Vec<Conn> {
    std::mem::take(&mut *NEW.lock().unwrap())
}

/// Accept overlay connections on `<pipe>-overlay`. `app_for` checks a
/// launch token (the same tokens as the steamclient pipe).
#[cfg(windows)]
pub async fn accept(pipe: String, app_for: impl Fn(&str) -> Option<AppId> + Send + Sync + 'static) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = wire::pipe_name(&pipe);
    let app_for = Arc::new(app_for);
    let mut first = true;
    loop {
        let server = match ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create(&name) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("overlay pipe {name}: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        first = false;
        if server.connect().await.is_err() {
            continue;
        }
        tokio::spawn(serve(server, app_for.clone()));
    }
}

#[cfg(windows)]
async fn read_message(r: &mut (impl tokio::io::AsyncRead + Unpin)) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let n = r.read_u32_le().await? as usize;
    if n == 0 || n > wire::MAX_MESSAGE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad overlay message"));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(body)
}

#[cfg(windows)]
async fn serve(pipe: tokio::net::windows::named_pipe::NamedPipeServer, app_for: Arc<dyn Fn(&str) -> Option<AppId> + Send + Sync>) {
    use tokio::io::AsyncWriteExt;
    let (mut r, mut w) = tokio::io::split(pipe);
    let hello = tokio::time::timeout(std::time::Duration::from_secs(10), read_message(&mut r)).await;
    let Ok(Ok(Ok(FromGame::Hello { token, pid, width, height }))) = hello.map(|m| m.map(|b| FromGame::decode(&b))) else { return };
    let Some(app_id) = app_for(&token) else {
        tracing::warn!("overlay: connection with an unknown launch token");
        return;
    };
    tracing::info!("in-game overlay: app {app_id} (pid {pid}) draws it, {width}x{height}");
    let (in_tx, in_rx) = std::sync::mpsc::channel();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    NEW.lock().unwrap().push(Conn { app_id, pid, size: (width, height), from_game: in_rx, to_game: out_tx });
    wake();
    let writer = tokio::spawn(async move {
        while let Some(body) = out_rx.recv().await {
            let mut buf = Vec::with_capacity(body.len() + 4);
            buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
            buf.extend_from_slice(&body);
            if w.write_all(&buf).await.is_err() || w.flush().await.is_err() {
                break;
            }
        }
    });
    while let Ok(body) = read_message(&mut r).await {
        match FromGame::decode(&body) {
            Ok(m) => {
                if in_tx.send(m).is_err() {
                    break;
                }
                wake();
            }
            Err(_) => break,
        }
    }
    writer.abort();
    // Dropping `in_tx` tells the UI the game is gone.
    drop(in_tx);
    wake();
}

//! macOS: how games find SteamInRust's steamclient.
//!
//! On Windows a game's `steam_api` reads the steamclient path from the
//! registry, and SteamInRust points it at its own DLL for one launch (see
//! `launch::swap_begin`). On macOS `libsteam_api.dylib` asks the Mach
//! service `com.valvesoftware.steam.ipctool` (Steam's `ipcserver`) for "the
//! running Steam": a process id it checks is alive and a file path, then
//! loads `steamclient.dylib` from that file's folder.
//!
//! So for a launch SteamInRust answers that service itself, with its own
//! process id and a folder holding its steamclient.dylib, and stops as soon
//! as the game has loaded it (or exits), like the Windows registry swap.
//! The name is only taken while nobody else has it: when the official
//! client's ipcserver runs, games keep talking to real Steam.
//!
//! Messages (all with `msgh_id` 104, the protocol version):
//! - request body `100001` (version) → reply of 32 bytes, version at 28;
//! - request body `14` (GetSteamPath) → reply of 544 bytes: pid at 28,
//!   path at 32;
//!
//! The client receives into 40- and 552-byte buffers, which must also hold
//! the 8-byte trailer the kernel adds, hence the reply sizes.
//! - request body `100000` (stop) → no reply.

#![allow(non_camel_case_types)]

use std::collections::HashSet;
use std::ffi::{CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SERVICE: &str = "com.valvesoftware.steam.ipctool";
const VERSION: u32 = 104;
const CMD_GET_STEAM_PATH: u32 = 14;
const CMD_VERSION: u32 = 100_001;

type mach_port_t = u32;
type kern_return_t = i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Header {
    bits: u32,
    size: u32,
    remote: mach_port_t,
    local: mach_port_t,
    voucher: mach_port_t,
    id: i32,
}

const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
/// The request makes a send right for its reply port (bits 0x1413), so the
/// reply moves that right back.
const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
const MACH_SEND_MSG: i32 = 1;
const MACH_RCV_MSG: i32 = 2;
const MACH_SEND_TIMEOUT: i32 = 0x10;
const MACH_RCV_TIMEOUT: i32 = 0x100;
const MACH_RCV_TIMED_OUT: kern_return_t = 0x1000_4003;

unsafe extern "C" {
    static mach_task_self_: mach_port_t;
    static bootstrap_port: mach_port_t;
    fn mach_port_allocate(task: mach_port_t, right: u32, name: *mut mach_port_t) -> kern_return_t;
    fn mach_port_insert_right(task: mach_port_t, name: mach_port_t, right: mach_port_t, kind: u32) -> kern_return_t;
    fn mach_port_mod_refs(task: mach_port_t, name: mach_port_t, right: u32, delta: i32) -> kern_return_t;
    fn mach_port_deallocate(task: mach_port_t, name: mach_port_t) -> kern_return_t;
    fn bootstrap_register(bp: mach_port_t, name: *const c_char, port: mach_port_t) -> kern_return_t;
    fn bootstrap_look_up(bp: mach_port_t, name: *const c_char, port: *mut mach_port_t) -> kern_return_t;
    fn mach_msg(msg: *mut Header, option: i32, send_size: u32, rcv_size: u32, rcv_name: mach_port_t, timeout: u32, notify: mach_port_t) -> kern_return_t;
}

struct Server {
    port: mach_port_t,
    /// Launches whose game hasn't loaded our steamclient yet.
    pending: HashSet<String>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// The folder libsteam_api loads `steamclient.dylib` from: ours, named the
/// way it wants (a link to wherever the library is).
fn client_dir(dylib: &Path) -> Result<PathBuf, String> {
    if dylib.file_name().is_some_and(|n| n == "steamclient.dylib") {
        return dylib.parent().map(Path::to_path_buf).ok_or_else(|| "bad steamclient path".to_string());
    }
    let dir = crate::store::config_dir().join("steamclient");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let link = dir.join("steamclient.dylib");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(dylib, &link).map_err(|e| format!("steamclient link: {e}"))?;
    Ok(dir)
}

/// Someone else (the official client's ipcserver) already answers.
fn taken() -> bool {
    let name = CString::new(SERVICE).unwrap();
    let mut port = 0;
    let found = unsafe { bootstrap_look_up(bootstrap_port, name.as_ptr(), &mut port) } == 0;
    if found {
        unsafe { mach_port_deallocate(mach_task_self_, port) };
    }
    found
}

/// Answer for one launch (until [`loaded`] or [`exited`] for `token`).
pub fn begin(token: &str, dylib: &Path) -> Result<(), String> {
    let mut server = SERVER.lock().unwrap();
    if let Some(s) = server.as_mut() {
        s.pending.insert(token.to_string());
        return Ok(());
    }
    if taken() {
        return Err("the official Steam client is running; the game will use it".into());
    }
    let path = client_dir(dylib)?.join("steam");
    let port = unsafe {
        let mut port = 0;
        if mach_port_allocate(mach_task_self_, MACH_PORT_RIGHT_RECEIVE, &mut port) != 0 {
            return Err("mach port".into());
        }
        mach_port_insert_right(mach_task_self_, port, port, MACH_MSG_TYPE_MAKE_SEND);
        let name = CString::new(SERVICE).unwrap();
        #[allow(deprecated)]
        let r = bootstrap_register(bootstrap_port, name.as_ptr(), port);
        if r != 0 {
            mach_port_mod_refs(mach_task_self_, port, MACH_PORT_RIGHT_RECEIVE, -1);
            return Err(format!("couldn't offer {SERVICE} ({r})"));
        }
        port
    };
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("ipctool".into()).spawn(move || serve(port, path, flag)).map_err(|e| e.to_string())?;
    tracing::info!("answering {SERVICE} for games started with SteamInRust's steamclient");
    *server = Some(Server { port, pending: [token.to_string()].into(), stop });
    Ok(())
}

/// The game loaded our steamclient (it said hello) or never will.
pub fn loaded(token: &str) {
    let mut server = SERVER.lock().unwrap();
    let done = server.as_mut().is_some_and(|s| {
        s.pending.remove(token);
        s.pending.is_empty()
    });
    if done {
        shut(server.take());
    }
}

/// The game closed.
pub fn exited(token: &str) {
    loaded(token);
}

/// Stop answering now.
pub fn stop() {
    shut(SERVER.lock().unwrap().take());
}

fn shut(server: Option<Server>) {
    if let Some(s) = server {
        s.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        // Dropping the receive right removes the name from the bootstrap.
        unsafe {
            mach_port_mod_refs(mach_task_self_, s.port, MACH_PORT_RIGHT_RECEIVE, -1);
        }
        tracing::info!("stopped answering {SERVICE}");
    }
}

fn serve(port: mach_port_t, path: PathBuf, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let pid = std::process::id();
    let path = path.to_string_lossy().into_owned();
    let mut buf = [0u8; 1024];
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let msg = buf.as_mut_ptr() as *mut Header;
        let r = unsafe { mach_msg(msg, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, buf.len() as u32, port, 500, 0) };
        if r == MACH_RCV_TIMED_OUT {
            continue;
        }
        if r != 0 {
            // The port went away (stop) or broke.
            return;
        }
        let header = unsafe { msg.read_unaligned() };
        let command = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        if let Some(reply) = answer(command, pid, &path) {
            send(header.remote, reply);
        } else if header.remote != 0 {
            unsafe { mach_port_deallocate(mach_task_self_, header.remote) };
        }
    }
}

/// The reply body (after the 24-byte header) for a request, if it gets one.
fn answer(command: u32, pid: u32, path: &str) -> Option<Vec<u8>> {
    match command {
        CMD_VERSION => {
            let mut b = vec![0u8; 32];
            b[28..32].copy_from_slice(&VERSION.to_le_bytes());
            Some(b)
        }
        CMD_GET_STEAM_PATH => {
            let mut b = vec![0u8; 544];
            b[24..28].copy_from_slice(&CMD_GET_STEAM_PATH.to_le_bytes());
            b[28..32].copy_from_slice(&pid.to_le_bytes());
            let n = path.len().min(544 - 32 - 1);
            b[32..32 + n].copy_from_slice(&path.as_bytes()[..n]);
            Some(b)
        }
        _ => None,
    }
}

fn send(reply_port: mach_port_t, mut msg: Vec<u8>) {
    if reply_port == 0 {
        return;
    }
    let header = Header { bits: MACH_MSG_TYPE_MOVE_SEND, size: msg.len() as u32, remote: reply_port, local: 0, voucher: 0, id: VERSION as i32 };
    unsafe {
        (msg.as_mut_ptr() as *mut Header).write_unaligned(header);
        mach_msg(msg.as_mut_ptr() as *mut Header, MACH_SEND_MSG | MACH_SEND_TIMEOUT, msg.len() as u32, 0, 0, 1000, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let v = answer(CMD_VERSION, 1, "x").unwrap();
        assert_eq!((v.len(), u32::from_le_bytes(v[28..32].try_into().unwrap())), (32, 104));
        let p = answer(CMD_GET_STEAM_PATH, 4321, "/a/b/steam").unwrap();
        assert_eq!(p.len(), 544);
        assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()), 4321);
        assert_eq!(&p[32..42], b"/a/b/steam");
        assert_eq!(p[42], 0);
        assert!(answer(100_000, 1, "x").is_none());
    }

    /// A client like libsteam_api's: look the service up, ask, read the answer.
    #[test]
    fn answers_like_ipcserver() {
        let dir = std::env::temp_dir().join(format!("sir-ipctool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dylib = dir.join("steamclient.dylib");
        std::fs::write(&dylib, b"x").unwrap();
        if taken() {
            return; // Steam's own ipcserver is running on this machine.
        }
        begin("t", &dylib).unwrap();
        unsafe {
            let name = CString::new(SERVICE).unwrap();
            let mut server = 0;
            assert_eq!(bootstrap_look_up(bootstrap_port, name.as_ptr(), &mut server), 0);
            let mut reply_port = 0;
            mach_port_allocate(mach_task_self_, MACH_PORT_RIGHT_RECEIVE, &mut reply_port);
            let mut req = [0u8; 32];
            let h = Header { bits: 0x1413, size: 32, remote: server, local: reply_port, voucher: 0, id: 104 };
            (req.as_mut_ptr() as *mut Header).write_unaligned(h);
            req[24..28].copy_from_slice(&CMD_GET_STEAM_PATH.to_le_bytes());
            assert_eq!(mach_msg(req.as_mut_ptr() as *mut Header, MACH_SEND_MSG | MACH_SEND_TIMEOUT, 32, 0, 0, 2000, 0), 0);
            // The same buffer size libsteam_api receives into (552, trailer included).
            let mut rep = [0u8; 552];
            assert_eq!(mach_msg(rep.as_mut_ptr() as *mut Header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, 552, reply_port, 2000, 0), 0);
            assert_eq!(u32::from_le_bytes(rep[28..32].try_into().unwrap()), std::process::id());
            let path = std::ffi::CStr::from_ptr(rep[32..].as_ptr() as *const c_char).to_string_lossy().into_owned();
            assert_eq!(path, dir.join("steam").to_string_lossy());
            // The version check, into a 40-byte buffer like libsteam_api's.
            let h = Header { bits: 0x1413, size: 32, remote: server, local: reply_port, voucher: 0, id: 104 };
            (req.as_mut_ptr() as *mut Header).write_unaligned(h);
            req[24..28].copy_from_slice(&CMD_VERSION.to_le_bytes());
            assert_eq!(mach_msg(req.as_mut_ptr() as *mut Header, MACH_SEND_MSG | MACH_SEND_TIMEOUT, 32, 0, 0, 2000, 0), 0);
            let mut rep = [0u8; 40];
            assert_eq!(mach_msg(rep.as_mut_ptr() as *mut Header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, 40, reply_port, 2000, 0), 0);
            assert_eq!(u32::from_le_bytes(rep[28..32].try_into().unwrap()), VERSION);
        }
        loaded("t");
        assert!(!taken());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

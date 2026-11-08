//! Messages between the in-game overlay (inside the steamclient DLL) and the
//! SteamInRust app. Shared source: the app includes this file with `#[path]`,
//! like `protocol.rs`. Plain std, no egui: the DLL stays small.
//!
//! Transport: the named pipe `<ipc pipe>-overlay`, one connection per game
//! process, kept open. Every message is `u32 length` + `u8 kind` + payload,
//! little-endian.
//!
//! The DLL connects once the game's first frame went through our hook, so a
//! connection means "this game draws the overlay itself". The app then runs
//! the overlay's UI and sends what to draw (egui triangles and textures); the
//! DLL sends back the game's mouse and keyboard while the overlay is open.

use std::io::{self, Read, Write};

/// Pipe name for a game's overlay connection.
pub fn pipe_name(ipc_pipe: &str) -> String {
    format!("{ipc_pipe}-overlay")
}

/// Largest message accepted (a full font atlas fits easily).
pub const MAX_MESSAGE: usize = 256 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Pointer position in back-buffer pixels.
    Move { x: f32, y: f32 },
    /// 0 left, 1 right, 2 middle, 3 back, 4 forward.
    Button { button: u8, down: bool, x: f32, y: f32 },
    /// Wheel, in notches (120 units = 1.0); `y` up is positive.
    Wheel { x: f32, y: f32 },
    /// Windows virtual-key code.
    Key { vk: u32, down: bool, repeat: bool },
    Text(String),
    /// Clipboard text for Ctrl+V.
    Paste(String),
    /// The pointer left the window.
    Leave,
}

/// Modifier bits sent with every input batch.
pub const MOD_SHIFT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_ALT: u8 = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum FromGame {
    /// First message: who we are and the back buffer's size.
    Hello { token: String, pid: u32, width: u32, height: u32 },
    Resize { width: u32, height: u32 },
    /// Shift+Tab (or the game) opened or closed the overlay.
    Opened(bool),
    Input { modifiers: u8, events: Vec<Input> },
}

/// One texture change, in egui's terms (`TexturesDelta`).
#[derive(Debug, Clone, PartialEq)]
pub struct TexSet {
    pub id: u64,
    /// `None` replaces the whole texture, else a patch at this pixel.
    pub pos: Option<[u32; 2]>,
    pub size: [u32; 2],
    pub linear: bool,
    /// Premultiplied sRGBA, `size[0] * size[1] * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Triangles with one texture and one clip rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    /// min x, min y, max x, max y, in points.
    pub clip: [f32; 4],
    pub texture: u64,
    /// `VERTEX_SIZE` bytes each: pos.x, pos.y, u, v (f32), then RGBA bytes.
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
}

pub const VERTEX_SIZE: usize = 20;

/// Mouse cursor shapes the overlay asks for.
pub mod cursor {
    pub const ARROW: u8 = 0;
    pub const HAND: u8 = 1;
    pub const TEXT: u8 = 2;
    pub const RESIZE_H: u8 = 3;
    pub const RESIZE_V: u8 = 4;
    pub const RESIZE_NWSE: u8 = 5;
    pub const RESIZE_NESW: u8 = 6;
    pub const MOVE: u8 = 7;
    pub const NOT_ALLOWED: u8 = 8;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToGame {
    /// What to draw from now on. Textures are applied in order, before the
    /// meshes; an empty mesh list draws nothing.
    Frame { pixels_per_point: f32, cursor: u8, set: Vec<TexSet>, free: Vec<u64>, meshes: Vec<Mesh> },
    /// Open or close the overlay (its close button, Esc, the game asking).
    SetOpen(bool),
}

const HELLO: u8 = 1;
const RESIZE: u8 = 2;
const OPENED: u8 = 3;
const INPUT: u8 = 4;
const FRAME: u8 = 10;
const SET_OPEN: u8 = 11;

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
}

struct R<'a>(&'a [u8]);

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "bad overlay message")
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(bad());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> io::Result<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    /// A count of items at least `min` bytes each (guards huge allocations).
    fn count(&mut self, min: usize) -> io::Result<usize> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min.max(1)) > self.0.len() {
            return Err(bad());
        }
        Ok(n)
    }
}

impl FromGame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            FromGame::Hello { token, pid, width, height } => {
                w.u8(HELLO);
                w.str(token);
                w.u32(*pid);
                w.u32(*width);
                w.u32(*height);
            }
            FromGame::Resize { width, height } => {
                w.u8(RESIZE);
                w.u32(*width);
                w.u32(*height);
            }
            FromGame::Opened(open) => {
                w.u8(OPENED);
                w.u8(*open as u8);
            }
            FromGame::Input { modifiers, events } => {
                w.u8(INPUT);
                w.u8(*modifiers);
                w.u32(events.len() as u32);
                for e in events {
                    match e {
                        Input::Move { x, y } => {
                            w.u8(0);
                            w.f32(*x);
                            w.f32(*y);
                        }
                        Input::Button { button, down, x, y } => {
                            w.u8(1);
                            w.u8(*button);
                            w.u8(*down as u8);
                            w.f32(*x);
                            w.f32(*y);
                        }
                        Input::Wheel { x, y } => {
                            w.u8(2);
                            w.f32(*x);
                            w.f32(*y);
                        }
                        Input::Key { vk, down, repeat } => {
                            w.u8(3);
                            w.u32(*vk);
                            w.u8(*down as u8);
                            w.u8(*repeat as u8);
                        }
                        Input::Text(s) => {
                            w.u8(4);
                            w.str(s);
                        }
                        Input::Paste(s) => {
                            w.u8(5);
                            w.str(s);
                        }
                        Input::Leave => w.u8(6),
                    }
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> io::Result<Self> {
        let mut r = R(b);
        Ok(match r.u8()? {
            HELLO => FromGame::Hello { token: r.str()?, pid: r.u32()?, width: r.u32()?, height: r.u32()? },
            RESIZE => FromGame::Resize { width: r.u32()?, height: r.u32()? },
            OPENED => FromGame::Opened(r.u8()? != 0),
            INPUT => {
                let modifiers = r.u8()?;
                let n = r.count(1)?;
                let mut events = Vec::with_capacity(n);
                for _ in 0..n {
                    events.push(match r.u8()? {
                        0 => Input::Move { x: r.f32()?, y: r.f32()? },
                        1 => Input::Button { button: r.u8()?, down: r.u8()? != 0, x: r.f32()?, y: r.f32()? },
                        2 => Input::Wheel { x: r.f32()?, y: r.f32()? },
                        3 => Input::Key { vk: r.u32()?, down: r.u8()? != 0, repeat: r.u8()? != 0 },
                        4 => Input::Text(r.str()?),
                        5 => Input::Paste(r.str()?),
                        6 => Input::Leave,
                        _ => return Err(bad()),
                    });
                }
                FromGame::Input { modifiers, events }
            }
            _ => return Err(bad()),
        })
    }
}

impl ToGame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            ToGame::Frame { pixels_per_point, cursor, set, free, meshes } => {
                w.u8(FRAME);
                w.f32(*pixels_per_point);
                w.u8(*cursor);
                w.u32(set.len() as u32);
                for t in set {
                    w.u64(t.id);
                    match t.pos {
                        Some([x, y]) => {
                            w.u8(1);
                            w.u32(x);
                            w.u32(y);
                        }
                        None => w.u8(0),
                    }
                    w.u32(t.size[0]);
                    w.u32(t.size[1]);
                    w.u8(t.linear as u8);
                    w.bytes(&t.rgba);
                }
                w.u32(free.len() as u32);
                for f in free {
                    w.u64(*f);
                }
                w.u32(meshes.len() as u32);
                for m in meshes {
                    for c in m.clip {
                        w.f32(c);
                    }
                    w.u64(m.texture);
                    w.bytes(&m.vertices);
                    w.u32(m.indices.len() as u32);
                    for i in &m.indices {
                        w.u32(*i);
                    }
                }
            }
            ToGame::SetOpen(open) => {
                w.u8(SET_OPEN);
                w.u8(*open as u8);
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> io::Result<Self> {
        let mut r = R(b);
        Ok(match r.u8()? {
            FRAME => {
                let pixels_per_point = r.f32()?;
                let cursor = r.u8()?;
                let n = r.count(8 + 1 + 8 + 1 + 4)?;
                let mut set = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u64()?;
                    let pos = if r.u8()? != 0 { Some([r.u32()?, r.u32()?]) } else { None };
                    let size = [r.u32()?, r.u32()?];
                    let linear = r.u8()? != 0;
                    let rgba = r.bytes()?.to_vec();
                    if rgba.len() as u64 != size[0] as u64 * size[1] as u64 * 4 {
                        return Err(bad());
                    }
                    set.push(TexSet { id, pos, size, linear, rgba });
                }
                let n = r.count(8)?;
                let mut free = Vec::with_capacity(n);
                for _ in 0..n {
                    free.push(r.u64()?);
                }
                let n = r.count(16 + 8 + 8)?;
                let mut meshes = Vec::with_capacity(n);
                for _ in 0..n {
                    let clip = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                    let texture = r.u64()?;
                    let vertices = r.bytes()?.to_vec();
                    if vertices.len() % VERTEX_SIZE != 0 {
                        return Err(bad());
                    }
                    let k = r.count(4)?;
                    let raw = r.take(k * 4)?;
                    let indices = raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
                    meshes.push(Mesh { clip, texture, vertices, indices });
                }
                ToGame::Frame { pixels_per_point, cursor, set, free, meshes }
            }
            SET_OPEN => ToGame::SetOpen(r.u8()? != 0),
            _ => return Err(bad()),
        })
    }
}

/// Write one message (length prefix + body).
pub fn write_message(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(body.len() + 4);
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one message body.
pub fn read_message(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_MESSAGE {
        return Err(bad());
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let msgs = [
            FromGame::Hello { token: "abc".into(), pid: 7, width: 1920, height: 1080 },
            FromGame::Resize { width: 1, height: 2 },
            FromGame::Opened(true),
            FromGame::Input {
                modifiers: MOD_SHIFT | MOD_CTRL,
                events: vec![
                    Input::Move { x: 1.5, y: 2.5 },
                    Input::Button { button: 1, down: true, x: 3.0, y: 4.0 },
                    Input::Wheel { x: 0.0, y: -1.0 },
                    Input::Key { vk: 0x41, down: false, repeat: true },
                    Input::Text("привет".into()),
                    Input::Paste("x".into()),
                    Input::Leave,
                ],
            },
        ];
        for m in msgs {
            assert_eq!(FromGame::decode(&m.encode()).unwrap(), m);
        }
        let frame = ToGame::Frame {
            pixels_per_point: 1.25,
            cursor: cursor::TEXT,
            set: vec![TexSet { id: 3, pos: Some([4, 5]), size: [1, 2], linear: true, rgba: vec![1; 8] }],
            free: vec![9],
            meshes: vec![Mesh { clip: [0.0, 0.0, 10.0, 10.0], texture: 3, vertices: vec![0; VERTEX_SIZE * 3], indices: vec![0, 1, 2] }],
        };
        assert_eq!(ToGame::decode(&frame.encode()).unwrap(), frame);
        assert_eq!(ToGame::decode(&ToGame::SetOpen(false).encode()).unwrap(), ToGame::SetOpen(false));
        let mut pipe = Vec::new();
        write_message(&mut pipe, &frame.encode()).unwrap();
        assert_eq!(ToGame::decode(&read_message(&mut &pipe[..]).unwrap()).unwrap(), frame);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ToGame::decode(&[FRAME, 0, 0]).is_err());
        assert!(FromGame::decode(&[INPUT, 0, 255, 255, 255, 255]).is_err());
        assert!(read_message(&mut &[0u8, 0, 0, 0][..]).is_err());
    }
}

//! Drawing the overlay inside a D3D11 game.
//!
//! We make a throwaway device and swap chain on a hidden window to find
//! DXGI's swap chain method table, then point `Present` (and `Present1`,
//! `ResizeBuffers`, `ResizeBuffers1`) at our functions. Every swap chain in
//! the process shares that table, so the game's own frames come through
//! here: we draw the overlay's triangles on top of the back buffer, put the
//! game's D3D11 state back exactly as it was, and call the real `Present`.
//!
//! D3D12 swap chains arrive through the same hook and go to `d3d12.rs`.
//! Games on anything else never send a frame here, so the app keeps its
//! window overlay for them.

use super::{Frame, TexOp, shared};
use crate::overlay::wire::VERTEX_SIZE;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{HRESULT, Interface, PCSTR, s, w};

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type Present1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32) -> HRESULT;
type Resize1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32, *const u32, *const *mut c_void) -> HRESULT;

static REAL_PRESENT: AtomicUsize = AtomicUsize::new(0);
static REAL_PRESENT1: AtomicUsize = AtomicUsize::new(0);
static REAL_RESIZE: AtomicUsize = AtomicUsize::new(0);
static REAL_RESIZE1: AtomicUsize = AtomicUsize::new(0);

/// IDXGISwapChain method slots (IUnknown 0-2, IDXGIObject 3-6, GetDevice 7).
const SLOT_PRESENT: usize = 8;
const SLOT_RESIZE: usize = 13;
const SLOT_PRESENT1: usize = 22;
const SLOT_RESIZE1: usize = 39;

/// Point DXGI's swap chain methods at ours. Safe to call more than once.
pub fn install() -> bool {
    if REAL_PRESENT.load(Ordering::Acquire) != 0 {
        return true;
    }
    match unsafe { find_vtable() } {
        Some((vtable, has1, has3)) => unsafe {
            patch(vtable, SLOT_RESIZE, resize as *const () as usize, &REAL_RESIZE);
            if has3 {
                patch(vtable, SLOT_RESIZE1, resize1 as *const () as usize, &REAL_RESIZE1);
            }
            if has1 {
                patch(vtable, SLOT_PRESENT1, present1 as *const () as usize, &REAL_PRESENT1);
            }
            patch(vtable, SLOT_PRESENT, present as *const () as usize, &REAL_PRESENT);
            crate::log!("overlay: D3D11 hooks in");
            true
        },
        None => {
            crate::log!("overlay: no D3D11 here, not hooking");
            false
        }
    }
}

pub(super) unsafe fn patch(vtable: *mut usize, slot: usize, ours: usize, real: &AtomicUsize) {
    use windows::Win32::System::Memory::{PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect};
    unsafe {
        let p = vtable.add(slot);
        if *p == ours {
            return;
        }
        let mut old = PAGE_PROTECTION_FLAGS(0);
        if VirtualProtect(p as *const c_void, size_of::<usize>(), PAGE_READWRITE, &mut old).is_err() {
            crate::log!("overlay: VirtualProtect failed for slot {slot}");
            return;
        }
        real.store(*p, Ordering::Release);
        *p = ours;
        let _ = VirtualProtect(p as *const c_void, size_of::<usize>(), old, &mut old);
    }
}

/// A throwaway swap chain's method table, and whether it is also
/// IDXGISwapChain1 and IDXGISwapChain3 (so those slots exist).
unsafe fn find_vtable() -> Option<(*mut usize, bool, bool)> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED};
    type CreateFn = unsafe extern "system" fn(*mut c_void, D3D_DRIVER_TYPE, HMODULE, u32, *const D3D_FEATURE_LEVEL, u32, u32, *const DXGI_SWAP_CHAIN_DESC, *mut *mut c_void, *mut *mut c_void, *mut D3D_FEATURE_LEVEL, *mut *mut c_void) -> HRESULT;
    unsafe {
        let module = LoadLibraryW(w!("d3d11.dll")).ok()?;
        let create: CreateFn = std::mem::transmute(GetProcAddress(module, s!("D3D11CreateDeviceAndSwapChain"))?);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("SteamInRust overlay probe"), WS_OVERLAPPED, 0, 0, 64, 64, None, None, None, None).ok()?;
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC { Width: 64, Height: 64, Format: DXGI_FORMAT_R8G8B8A8_UNORM, ..Default::default() },
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            OutputWindow: hwnd,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };
        let mut found = None;
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let (mut sc, mut dev, mut ctx) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            let hr = create(std::ptr::null_mut(), driver, HMODULE::default(), 0, std::ptr::null(), 0, D3D11_SDK_VERSION, &desc, &mut sc, &mut dev, std::ptr::null_mut(), &mut ctx);
            if hr.is_ok() && !sc.is_null() {
                let sc = IDXGISwapChain::from_raw(sc);
                let _dev = (!dev.is_null()).then(|| ID3D11Device::from_raw(dev));
                let _ctx = (!ctx.is_null()).then(|| ID3D11DeviceContext::from_raw(ctx));
                let vtable = *(sc.as_raw() as *mut *mut usize);
                let has1 = sc.cast::<IDXGISwapChain1>().is_ok();
                let has3 = sc.cast::<IDXGISwapChain3>().is_ok();
                found = Some((vtable, has1, has3));
                break;
            }
        }
        let _ = DestroyWindow(hwnd);
        found
    }
}

unsafe extern "system" fn present(this: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    let t0 = Instant::now();
    if flags & DXGI_PRESENT_TEST.0 == 0 {
        on_present(this);
    }
    let t1 = Instant::now();
    let real: PresentFn = unsafe { std::mem::transmute(REAL_PRESENT.load(Ordering::Acquire)) };
    let hr = unsafe { real(this, sync, flags) };
    stats::frame(t0, t1, Instant::now());
    hr
}

unsafe extern "system" fn present1(this: *mut c_void, sync: u32, flags: u32, params: *const c_void) -> HRESULT {
    let t0 = Instant::now();
    if flags & DXGI_PRESENT_TEST.0 == 0 {
        on_present(this);
    }
    let t1 = Instant::now();
    let real: Present1Fn = unsafe { std::mem::transmute(REAL_PRESENT1.load(Ordering::Acquire)) };
    let hr = unsafe { real(this, sync, flags, params) };
    stats::frame(t0, t1, Instant::now());
    hr
}

/// Frame timing, logged once a second when the DLL log is on
/// (`STEAMINRUST_STEAMCLIENT_LOG`): how long the game's frames take, how
/// much of that is our hook, and how long the real Present blocks.
pub(super) mod stats {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    struct Window {
        start: Instant,
        last: Option<Instant>,
        frames: u32,
        gap_sum: Duration,
        gap_max: Duration,
        hook_sum: Duration,
        hook_max: Duration,
        present_sum: Duration,
        present_max: Duration,
    }

    static W: Mutex<Option<Window>> = Mutex::new(None);
    /// Texture bytes uploaded and app frames drawn, since the last line.
    pub static TEX_BYTES: AtomicU64 = AtomicU64::new(0);
    pub static TEX_OPS: AtomicU64 = AtomicU64::new(0);
    pub static APP_FRAMES: AtomicU64 = AtomicU64::new(0);
    pub static UPLOADS: AtomicU64 = AtomicU64::new(0);

    fn on() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os(crate::protocol::ENV_LOG).is_some())
    }

    pub fn frame(t0: Instant, t1: Instant, t2: Instant) {
        if !on() {
            return;
        }
        let Ok(mut g) = W.lock() else { return };
        let w = g.get_or_insert_with(|| Window { start: t0, last: None, frames: 0, gap_sum: Duration::ZERO, gap_max: Duration::ZERO, hook_sum: Duration::ZERO, hook_max: Duration::ZERO, present_sum: Duration::ZERO, present_max: Duration::ZERO });
        if let Some(last) = w.last {
            let gap = t0 - last;
            w.gap_sum += gap;
            w.gap_max = w.gap_max.max(gap);
        }
        w.last = Some(t0);
        w.frames += 1;
        w.hook_sum += t1 - t0;
        w.hook_max = w.hook_max.max(t1 - t0);
        w.present_sum += t2 - t1;
        w.present_max = w.present_max.max(t2 - t1);
        if t0 - w.start >= Duration::from_secs(1) {
            let n = w.frames.max(1);
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            crate::log!(
                "overlay stats: {} fps, frame avg {:.1} ms max {:.1} ms, our hook avg {:.2} ms max {:.2} ms, real Present avg {:.2} ms max {:.2} ms, open {}, app frames {}, uploads {}, textures {} ({} KB)",
                n,
                ms(w.gap_sum) / (n.saturating_sub(1).max(1)) as f64,
                ms(w.gap_max),
                ms(w.hook_sum) / n as f64,
                ms(w.hook_max),
                ms(w.present_sum) / n as f64,
                ms(w.present_max),
                super::super::input::is_open(),
                APP_FRAMES.swap(0, Ordering::Relaxed),
                UPLOADS.swap(0, Ordering::Relaxed),
                TEX_OPS.swap(0, Ordering::Relaxed),
                TEX_BYTES.swap(0, Ordering::Relaxed) / 1024,
            );
            *g = Some(Window { start: t0, last: Some(t0), frames: 0, gap_sum: Duration::ZERO, gap_max: Duration::ZERO, hook_sum: Duration::ZERO, hook_max: Duration::ZERO, present_sum: Duration::ZERO, present_max: Duration::ZERO });
        }
    }
}

unsafe extern "system" fn resize(this: *mut c_void, count: u32, w: u32, h: u32, format: i32, flags: u32) -> HRESULT {
    before_resize(this);
    let real: ResizeFn = unsafe { std::mem::transmute(REAL_RESIZE.load(Ordering::Acquire)) };
    unsafe { real(this, count, w, h, format, flags) }
}

unsafe extern "system" fn resize1(this: *mut c_void, count: u32, w: u32, h: u32, format: i32, flags: u32, mask: *const u32, queues: *const *mut c_void) -> HRESULT {
    before_resize(this);
    let real: Resize1Fn = unsafe { std::mem::transmute(REAL_RESIZE1.load(Ordering::Acquire)) };
    unsafe { real(this, count, w, h, format, flags, mask, queues) }
}

/// The back buffer can't be resized while we hold a view of it.
fn before_resize(this: *mut c_void) {
    super::d3d12::before_resize(this as usize);
    if let Ok(mut r) = RENDERER.lock() {
        if let Some(r) = r.as_mut().filter(|r| r.swapchain == this as usize) {
            r.rtv = None;
        }
    }
}

struct Tex {
    _texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    size: [u32; 2],
    linear: bool,
}

struct Renderer {
    swapchain: usize,
    hwnd: HWND,
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    constants: ID3D11Buffer,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    depth: ID3D11DepthStencilState,
    linear_sampler: ID3D11SamplerState,
    nearest_sampler: ID3D11SamplerState,
    vb: Option<(ID3D11Buffer, usize)>,
    ib: Option<(ID3D11Buffer, usize)>,
    rtv: Option<ID3D11RenderTargetView>,
    fps: super::Fps,
    /// The render target stores linear light (sRGB view or float), so
    /// egui's gamma-space colors are converted in the shader.
    linear_out: bool,
    size: (u32, u32),
    textures: HashMap<u64, Tex>,
    /// The frame (and target size, color mode) whose triangles are in the buffers.
    uploaded: Option<(Arc<Frame>, (u32, u32), bool)>,
    last_present: Instant,
}

// COM pointers used only under RENDERER's lock, from whatever thread presents.
unsafe impl Send for Renderer {}

static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
/// Swap chains that turned out not to be D3D11 (e.g. D3D12).
static NOT_D3D11: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn on_present(this: *mut c_void) {
    // Never let a bug of ours take the game down.
    let _ = std::panic::catch_unwind(|| unsafe { present_inner(this) });
}

unsafe fn present_inner(this: *mut c_void) {
    let Some(sc) = (unsafe { IDXGISwapChain::from_raw_borrowed(&this) }) else { return };
    // D3D12 swap chains come through the same hook.
    if super::d3d12::on_present(sc, this as usize) {
        return;
    }
    let Ok(mut guard) = RENDERER.lock() else { return };
    if let Some(r) = guard.as_ref() {
        if r.swapchain != this as usize {
            // Another swap chain: only take over once ours went quiet (the
            // game recreated it, e.g. switching fullscreen).
            if r.last_present.elapsed().as_secs_f32() < 1.0 {
                return;
            }
            crate::log!("overlay: following a new swap chain");
            *guard = None;
        }
    }
    if guard.is_none() {
        if NOT_D3D11.lock().map(|l| l.contains(&(this as usize))).unwrap_or(true) {
            return;
        }
        match unsafe { Renderer::new(sc, this as usize) } {
            Ok(r) => {
                crate::log!("overlay: drawing into the game's D3D11 swap chain ({}x{})", r.size.0, r.size.1);
                super::input::attach(r.hwnd);
                super::hooked(r.size);
                *guard = Some(r);
            }
            Err(e) => {
                crate::log!("overlay: swap chain isn't usable: {e}");
                if let Ok(mut l) = NOT_D3D11.lock() {
                    l.push(this as usize);
                }
                return;
            }
        }
    }
    let r = guard.as_mut().expect("just set");
    r.last_present = Instant::now();
    if let Ok(desc) = unsafe { sc.GetDesc() } {
        let size = (desc.BufferDesc.Width, desc.BufferDesc.Height);
        if size != r.size {
            r.size = size;
            r.rtv = None;
            super::resized(size);
        }
        if desc.OutputWindow != r.hwnd {
            r.hwnd = desc.OutputWindow;
            super::input::attach(r.hwnd);
        }
    }
    // Frames per second, for the app's FPS counter.
    r.fps.tick();
    // The game's own frame, before the overlay is drawn on it.
    if super::SHOT.swap(false, Ordering::Relaxed) {
        match unsafe { r.capture(sc) } {
            Ok((rgb, w, h)) => crate::screenshots::save_frame(rgb, w, h),
            Err(e) => {
                crate::log!("overlay: screenshot copy failed: {e}");
                crate::screenshots::capture_window();
            }
        }
    }
    let (ops, frame) = shared::take_for_present();
    for op in ops {
        unsafe { r.apply(op) };
    }
    if let Some(frame) = frame.filter(|f| !f.meshes.is_empty()) {
        if let Err(e) = unsafe { r.draw(sc, &frame) } {
            crate::log!("overlay: draw failed: {e}");
        }
    }
}

const SHADER: &str = r#"
cbuffer C : register(b0) { float2 screen; float linear_out; float pad; };
struct VI { float2 pos : POSITION; float2 uv : TEXCOORD0; float4 col : COLOR0; };
struct PI { float4 pos : SV_POSITION; float2 uv : TEXCOORD0; float4 col : COLOR0; };
PI vs(VI i) {
    PI o;
    o.pos = float4(2.0 * i.pos.x / screen.x - 1.0, 1.0 - 2.0 * i.pos.y / screen.y, 0.0, 1.0);
    o.uv = i.uv;
    o.col = i.col;
    return o;
}
Texture2D t0 : register(t0);
SamplerState s0 : register(s0);
float3 to_linear(float3 c) { return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }
float4 ps(PI i) : SV_Target {
    float4 c = i.col * t0.Sample(s0, i.uv);
    if (linear_out > 0.5) c.rgb = to_linear(c.rgb);
    return c;
}
"#;

/// Compile one entry point with d3dcompiler_47 (loaded when needed, so
/// games that never draw the overlay don't load it).
pub(super) unsafe fn compile_source(source: &str, entry: PCSTR, target: PCSTR) -> windows::core::Result<Vec<u8>> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type CompileFn = unsafe extern "system" fn(*const c_void, usize, PCSTR, *const c_void, *mut c_void, PCSTR, PCSTR, u32, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
    unsafe {
        let module = LoadLibraryW(w!("d3dcompiler_47.dll"))?;
        let Some(f) = GetProcAddress(module, s!("D3DCompile")) else { return Err(windows::core::Error::from_thread()) };
        let f: CompileFn = std::mem::transmute(f);
        let (mut code, mut errors) = (std::ptr::null_mut(), std::ptr::null_mut());
        let hr = f(source.as_ptr().cast(), source.len(), PCSTR::null(), std::ptr::null(), std::ptr::null_mut(), entry, target, 0, 0, &mut code, &mut errors);
        if !errors.is_null() {
            let e = ID3DBlob::from_raw(errors);
            let text = std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize());
            crate::log!("overlay: shader: {}", String::from_utf8_lossy(text));
        }
        hr.ok()?;
        let code = ID3DBlob::from_raw(code);
        Ok(std::slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize()).to_vec())
    }
}

pub(super) fn out<T>(slot: Option<T>) -> windows::core::Result<T> {
    slot.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))
}

impl Renderer {
    unsafe fn new(sc: &IDXGISwapChain, raw: usize) -> windows::core::Result<Self> {
        unsafe {
            let device: ID3D11Device = sc.GetDevice()?;
            let ctx = device.GetImmediateContext()?;
            let desc = sc.GetDesc()?;
            let vs_code = compile_source(SHADER, s!("vs"), s!("vs_4_0"))?;
            let ps_code = compile_source(SHADER, s!("ps"), s!("ps_4_0"))?;
            let mut vs = None;
            device.CreateVertexShader(&vs_code, None, Some(&mut vs))?;
            let mut ps = None;
            device.CreatePixelShader(&ps_code, None, Some(&mut ps))?;
            let elements = [
                D3D11_INPUT_ELEMENT_DESC { SemanticName: s!("POSITION"), SemanticIndex: 0, Format: DXGI_FORMAT_R32G32_FLOAT, InputSlot: 0, AlignedByteOffset: 0, InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
                D3D11_INPUT_ELEMENT_DESC { SemanticName: s!("TEXCOORD"), SemanticIndex: 0, Format: DXGI_FORMAT_R32G32_FLOAT, InputSlot: 0, AlignedByteOffset: 8, InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
                D3D11_INPUT_ELEMENT_DESC { SemanticName: s!("COLOR"), SemanticIndex: 0, Format: DXGI_FORMAT_R8G8B8A8_UNORM, InputSlot: 0, AlignedByteOffset: 16, InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
            ];
            let mut layout = None;
            device.CreateInputLayout(&elements, &vs_code, Some(&mut layout))?;
            let cb_desc = D3D11_BUFFER_DESC { ByteWidth: 16, Usage: D3D11_USAGE_DYNAMIC, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..Default::default() };
            let mut constants = None;
            device.CreateBuffer(&cb_desc, None, Some(&mut constants))?;
            // Premultiplied alpha, like egui's own backends.
            let mut bd = D3D11_BLEND_DESC::default();
            bd.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
                DestBlendAlpha: D3D11_BLEND_ONE,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            device.CreateBlendState(&bd, Some(&mut blend))?;
            let rd = D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, ScissorEnable: true.into(), DepthClipEnable: true.into(), ..Default::default() };
            let mut raster = None;
            device.CreateRasterizerState(&rd, Some(&mut raster))?;
            let dd = D3D11_DEPTH_STENCIL_DESC { DepthEnable: false.into(), DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL, DepthFunc: D3D11_COMPARISON_ALWAYS, StencilEnable: false.into(), ..Default::default() };
            let mut depth = None;
            device.CreateDepthStencilState(&dd, Some(&mut depth))?;
            let sampler = |filter| -> windows::core::Result<ID3D11SamplerState> {
                let sd = D3D11_SAMPLER_DESC {
                    Filter: filter,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                };
                let mut s = None;
                device.CreateSamplerState(&sd, Some(&mut s))?;
                out(s)
            };
            Ok(Renderer {
                swapchain: raw,
                hwnd: desc.OutputWindow,
                linear_sampler: sampler(D3D11_FILTER_MIN_MAG_MIP_LINEAR)?,
                nearest_sampler: sampler(D3D11_FILTER_MIN_MAG_MIP_POINT)?,
                device,
                ctx,
                vs: out(vs)?,
                ps: out(ps)?,
                layout: out(layout)?,
                constants: out(constants)?,
                blend: out(blend)?,
                raster: out(raster)?,
                depth: out(depth)?,
                vb: None,
                ib: None,
                rtv: None,
                fps: super::Fps::new(),
                linear_out: false,
                size: (desc.BufferDesc.Width, desc.BufferDesc.Height),
                textures: HashMap::new(),
                uploaded: None,
                last_present: Instant::now(),
            })
        }
    }

    unsafe fn apply(&mut self, op: TexOp) {
        match op {
            TexOp::Free(id) => {
                self.textures.remove(&id);
            }
            TexOp::Set(t) => unsafe {
                stats::TEX_OPS.fetch_add(1, Ordering::Relaxed);
                stats::TEX_BYTES.fetch_add(t.rgba.len() as u64, Ordering::Relaxed);
                let [w, h] = t.size;
                if w == 0 || h == 0 {
                    return;
                }
                match t.pos {
                    Some([x, y]) => {
                        let Some(tex) = self.textures.get_mut(&t.id) else { return };
                        if x + w > tex.size[0] || y + h > tex.size[1] {
                            return;
                        }
                        tex.linear = t.linear;
                        let b = D3D11_BOX { left: x, top: y, front: 0, right: x + w, bottom: y + h, back: 1 };
                        let res: ID3D11Resource = match tex._texture.cast() {
                            Ok(r) => r,
                            Err(_) => return,
                        };
                        self.ctx.UpdateSubresource(&res, 0, Some(&b), t.rgba.as_ptr().cast(), w * 4, 0);
                    }
                    None => {
                        let desc = D3D11_TEXTURE2D_DESC {
                            Width: w,
                            Height: h,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Usage: D3D11_USAGE_DEFAULT,
                            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                            ..Default::default()
                        };
                        let data = D3D11_SUBRESOURCE_DATA { pSysMem: t.rgba.as_ptr().cast(), SysMemPitch: w * 4, SysMemSlicePitch: 0 };
                        let mut texture = None;
                        if self.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)).is_err() {
                            return;
                        }
                        let Some(texture) = texture else { return };
                        let mut view = None;
                        if self.device.CreateShaderResourceView(&texture, None, Some(&mut view)).is_err() {
                            return;
                        }
                        let Some(view) = view else { return };
                        self.textures.insert(t.id, Tex { _texture: texture, view, size: t.size, linear: t.linear });
                    }
                }
            },
        }
    }

    /// The back buffer as RGB (8 bits per channel), for a screenshot.
    unsafe fn capture(&self, sc: &IDXGISwapChain) -> windows::core::Result<(Vec<u8>, u32, u32)> {
        unsafe {
            let bb: ID3D11Texture2D = sc.GetBuffer(0)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            bb.GetDesc(&mut desc);
            let kind = pixel_kind(desc.Format).ok_or_else(|| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "unsupported back buffer format"))?;
            // Multisampled buffers are resolved into a plain texture first.
            let source: ID3D11Texture2D = if desc.SampleDesc.Count > 1 {
                let mut d = desc;
                d.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
                d.BindFlags = 0;
                d.CPUAccessFlags = 0;
                d.MiscFlags = 0;
                d.Usage = D3D11_USAGE_DEFAULT;
                let mut t = None;
                self.device.CreateTexture2D(&d, None, Some(&mut t))?;
                let t = out(t)?;
                self.ctx.ResolveSubresource(&t, 0, &bb, 0, typed(desc.Format));
                t
            } else {
                bb
            };
            let mut d = desc;
            d.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
            d.Usage = D3D11_USAGE_STAGING;
            d.BindFlags = 0;
            d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            d.MiscFlags = 0;
            d.MipLevels = 1;
            d.ArraySize = 1;
            let mut staging = None;
            self.device.CreateTexture2D(&d, None, Some(&mut staging))?;
            let staging = out(staging)?;
            self.ctx.CopySubresourceRegion(&staging, 0, 0, 0, 0, &source, 0, None);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let (w, h) = (desc.Width, desc.Height);
            let bpp = if kind == PixelKind::Half { 8 } else { 4 };
            let mut rgb = Vec::with_capacity((w * h * 3) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), w as usize * bpp);
                to_rgb(kind, row, &mut rgb);
            }
            self.ctx.Unmap(&staging, 0);
            Ok((rgb, w, h))
        }
    }

    unsafe fn ensure_target(&mut self, sc: &IDXGISwapChain) -> windows::core::Result<()> {
        if self.rtv.is_some() {
            return Ok(());
        }
        unsafe {
            let bb: ID3D11Texture2D = sc.GetBuffer(0)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            bb.GetDesc(&mut desc);
            let f = desc.Format;
            // Typeless buffers need a concrete view format; we write
            // gamma-space colors through a UNORM view when we can.
            let (view_format, linear) = match f {
                DXGI_FORMAT_R8G8B8A8_TYPELESS => (Some(DXGI_FORMAT_R8G8B8A8_UNORM), false),
                DXGI_FORMAT_B8G8R8A8_TYPELESS => (Some(DXGI_FORMAT_B8G8R8A8_UNORM), false),
                DXGI_FORMAT_R10G10B10A2_TYPELESS => (Some(DXGI_FORMAT_R10G10B10A2_UNORM), false),
                DXGI_FORMAT_R16G16B16A16_TYPELESS => (Some(DXGI_FORMAT_R16G16B16A16_FLOAT), true),
                DXGI_FORMAT_R8G8B8A8_UNORM_SRGB | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB | DXGI_FORMAT_R16G16B16A16_FLOAT => (None, true),
                _ => (None, false),
            };
            let mut rtv = None;
            match view_format {
                Some(format) => {
                    let multi = desc.SampleDesc.Count > 1;
                    let vd = D3D11_RENDER_TARGET_VIEW_DESC {
                        Format: format,
                        ViewDimension: if multi { D3D11_RTV_DIMENSION_TEXTURE2DMS } else { D3D11_RTV_DIMENSION_TEXTURE2D },
                        ..Default::default()
                    };
                    self.device.CreateRenderTargetView(&bb, Some(&vd), Some(&mut rtv))?;
                }
                None => self.device.CreateRenderTargetView(&bb, None, Some(&mut rtv))?,
            }
            self.rtv = Some(out(rtv)?);
            self.linear_out = linear;
            self.size = (desc.Width, desc.Height);
            Ok(())
        }
    }

    /// A dynamic buffer at least `bytes` big.
    unsafe fn buffer(device: &ID3D11Device, slot: &mut Option<(ID3D11Buffer, usize)>, bytes: usize, bind: D3D11_BIND_FLAG) -> windows::core::Result<ID3D11Buffer> {
        if let Some((b, cap)) = slot.as_ref().filter(|(_, cap)| *cap >= bytes) {
            let _ = cap;
            return Ok(b.clone());
        }
        let cap = bytes.next_power_of_two().max(64 * 1024);
        let desc = D3D11_BUFFER_DESC { ByteWidth: cap as u32, Usage: D3D11_USAGE_DYNAMIC, BindFlags: bind.0 as u32, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..Default::default() };
        let mut b = None;
        unsafe { device.CreateBuffer(&desc, None, Some(&mut b))? };
        let b = out(b)?;
        *slot = Some((b.clone(), cap));
        Ok(b)
    }

    unsafe fn upload(&self, buffer: &ID3D11Buffer, parts: &mut dyn Iterator<Item = &[u8]>) -> windows::core::Result<()> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let mut at = mapped.pData as *mut u8;
            for p in parts {
                std::ptr::copy_nonoverlapping(p.as_ptr(), at, p.len());
                at = at.add(p.len());
            }
            self.ctx.Unmap(buffer, 0);
        }
        Ok(())
    }

    unsafe fn draw(&mut self, sc: &IDXGISwapChain, frame: &Arc<Frame>) -> windows::core::Result<()> {
        unsafe {
            self.ensure_target(sc)?;
            let vbytes: usize = frame.meshes.iter().map(|m| m.vertices.len()).sum();
            let ibytes: usize = frame.meshes.iter().map(|m| m.indices.len() * 4).sum();
            if vbytes == 0 || ibytes == 0 {
                return Ok(());
            }
            let grew = self.vb.as_ref().is_none_or(|(_, cap)| *cap < vbytes) || self.ib.as_ref().is_none_or(|(_, cap)| *cap < ibytes);
            let vb = Self::buffer(&self.device, &mut self.vb, vbytes, D3D11_BIND_VERTEX_BUFFER)?;
            let ib = Self::buffer(&self.device, &mut self.ib, ibytes, D3D11_BIND_INDEX_BUFFER)?;
            let ppp = frame.pixels_per_point.max(0.1);
            let (w, h) = (self.size.0 as f32, self.size.1 as f32);
            // The game presents far more often than the app sends frames:
            // upload a frame's triangles once, not on every Present.
            let same = !grew && self.uploaded.as_ref().is_some_and(|(f, size, linear)| Arc::ptr_eq(f, frame) && *size == self.size && *linear == self.linear_out);
            if !same {
                self.upload(&vb, &mut frame.meshes.iter().map(|m| m.vertices.as_slice()))?;
                let idx: Vec<&[u8]> = frame.meshes.iter().map(|m| std::slice::from_raw_parts(m.indices.as_ptr() as *const u8, m.indices.len() * 4)).collect();
                self.upload(&ib, &mut idx.into_iter())?;
                let constants = [w / ppp, h / ppp, if self.linear_out { 1.0 } else { 0.0 }, 0.0f32];
                let cb_bytes = std::slice::from_raw_parts(constants.as_ptr() as *const u8, 16);
                self.upload(&self.constants, &mut std::iter::once(cb_bytes))?;
                self.uploaded = Some((frame.clone(), self.size, self.linear_out));
                stats::UPLOADS.fetch_add(1, Ordering::Relaxed);
            }

            let saved = Saved::take(&self.ctx);
            let c = &self.ctx;
            c.OMSetRenderTargets(Some(&[self.rtv.clone()]), None);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w, Height: h, MinDepth: 0.0, MaxDepth: 1.0 }]));
            c.RSSetState(&self.raster);
            c.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            c.OMSetDepthStencilState(&self.depth, 0);
            c.IASetInputLayout(&self.layout);
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            let stride = VERTEX_SIZE as u32;
            let offset = 0u32;
            c.IASetVertexBuffers(0, 1, Some(&Some(vb)), Some(&stride), Some(&offset));
            c.IASetIndexBuffer(&ib, DXGI_FORMAT_R32_UINT, 0);
            c.VSSetShader(&self.vs, None);
            c.VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            c.PSSetShader(&self.ps, None);
            c.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            c.GSSetShader(None::<&ID3D11GeometryShader>, None);
            c.HSSetShader(None::<&ID3D11HullShader>, None);
            c.DSSetShader(None::<&ID3D11DomainShader>, None);

            let (mut first_index, mut base_vertex) = (0u32, 0i32);
            for m in &frame.meshes {
                let count = m.indices.len() as u32;
                let verts = (m.vertices.len() / VERTEX_SIZE) as i32;
                let clip = RECT {
                    left: (m.clip[0] * ppp).round().clamp(0.0, w) as i32,
                    top: (m.clip[1] * ppp).round().clamp(0.0, h) as i32,
                    right: (m.clip[2] * ppp).round().clamp(0.0, w) as i32,
                    bottom: (m.clip[3] * ppp).round().clamp(0.0, h) as i32,
                };
                if let Some(tex) = self.textures.get(&m.texture).filter(|_| clip.right > clip.left && clip.bottom > clip.top && count > 0) {
                    c.RSSetScissorRects(Some(&[clip]));
                    c.PSSetShaderResources(0, Some(&[Some(tex.view.clone())]));
                    c.PSSetSamplers(0, Some(&[Some(if tex.linear { self.linear_sampler.clone() } else { self.nearest_sampler.clone() })]));
                    c.DrawIndexed(count, first_index, base_vertex);
                }
                first_index += count;
                base_vertex += verts;
            }
            saved.restore(&self.ctx);
            Ok(())
        }
    }
}

const MAX_RECTS: usize = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as usize;
const MAX_INSTANCES: usize = 256;

/// The game's D3D11 state we change, so it can be put back.
struct Saved {
    scissors: Vec<RECT>,
    viewports: Vec<D3D11_VIEWPORT>,
    raster: Option<ID3D11RasterizerState>,
    blend: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    depth: Option<ID3D11DepthStencilState>,
    stencil_ref: u32,
    ps_srv: [Option<ID3D11ShaderResourceView>; 1],
    ps_sampler: [Option<ID3D11SamplerState>; 1],
    ps: Option<ID3D11PixelShader>,
    ps_inst: Vec<Option<ID3D11ClassInstance>>,
    ps_cb: [Option<ID3D11Buffer>; 1],
    vs: Option<ID3D11VertexShader>,
    vs_inst: Vec<Option<ID3D11ClassInstance>>,
    vs_cb: [Option<ID3D11Buffer>; 1],
    gs: Option<ID3D11GeometryShader>,
    hs: Option<ID3D11HullShader>,
    ds: Option<ID3D11DomainShader>,
    topology: D3D_PRIMITIVE_TOPOLOGY,
    ib: Option<ID3D11Buffer>,
    ib_format: DXGI_FORMAT,
    ib_offset: u32,
    vb: Option<ID3D11Buffer>,
    vb_stride: u32,
    vb_offset: u32,
    layout: Option<ID3D11InputLayout>,
    rtvs: [Option<ID3D11RenderTargetView>; 8],
    dsv: Option<ID3D11DepthStencilView>,
}

impl Saved {
    unsafe fn take(c: &ID3D11DeviceContext) -> Saved {
        unsafe {
            let mut s = Saved {
                scissors: vec![RECT::default(); MAX_RECTS],
                viewports: vec![D3D11_VIEWPORT::default(); MAX_RECTS],
                raster: c.RSGetState().ok(),
                blend: None,
                blend_factor: [0.0; 4],
                sample_mask: 0,
                depth: None,
                stencil_ref: 0,
                ps_srv: [None],
                ps_sampler: [None],
                ps: None,
                ps_inst: vec![None; MAX_INSTANCES],
                ps_cb: [None],
                vs: None,
                vs_inst: vec![None; MAX_INSTANCES],
                vs_cb: [None],
                gs: None,
                hs: None,
                ds: None,
                topology: c.IAGetPrimitiveTopology(),
                ib: None,
                ib_format: DXGI_FORMAT_UNKNOWN,
                ib_offset: 0,
                vb: None,
                vb_stride: 0,
                vb_offset: 0,
                layout: c.IAGetInputLayout().ok(),
                rtvs: Default::default(),
                dsv: None,
            };
            let mut n = MAX_RECTS as u32;
            c.RSGetScissorRects(&mut n, Some(s.scissors.as_mut_ptr()));
            s.scissors.truncate(n as usize);
            let mut n = MAX_RECTS as u32;
            c.RSGetViewports(&mut n, Some(s.viewports.as_mut_ptr()));
            s.viewports.truncate(n as usize);
            c.OMGetBlendState(Some(&mut s.blend), Some(&mut s.blend_factor), Some(&mut s.sample_mask));
            c.OMGetDepthStencilState(Some(&mut s.depth), Some(&mut s.stencil_ref));
            c.PSGetShaderResources(0, Some(&mut s.ps_srv));
            c.PSGetSamplers(0, Some(&mut s.ps_sampler));
            let mut n = MAX_INSTANCES as u32;
            c.PSGetShader(&mut s.ps, Some(s.ps_inst.as_mut_ptr()), Some(&mut n));
            s.ps_inst.truncate(n as usize);
            c.PSGetConstantBuffers(0, Some(&mut s.ps_cb));
            let mut n = MAX_INSTANCES as u32;
            c.VSGetShader(&mut s.vs, Some(s.vs_inst.as_mut_ptr()), Some(&mut n));
            s.vs_inst.truncate(n as usize);
            c.VSGetConstantBuffers(0, Some(&mut s.vs_cb));
            c.GSGetShader(&mut s.gs, None, None);
            c.HSGetShader(&mut s.hs, None, None);
            c.DSGetShader(&mut s.ds, None, None);
            c.IAGetIndexBuffer(Some(&mut s.ib), Some(&mut s.ib_format), Some(&mut s.ib_offset));
            c.IAGetVertexBuffers(0, 1, Some(&mut s.vb), Some(&mut s.vb_stride), Some(&mut s.vb_offset));
            c.OMGetRenderTargets(Some(&mut s.rtvs), Some(&mut s.dsv));
            s
        }
    }

    unsafe fn restore(self, c: &ID3D11DeviceContext) {
        unsafe {
            c.RSSetScissorRects(Some(&self.scissors));
            c.RSSetViewports(Some(&self.viewports));
            c.RSSetState(self.raster.as_ref());
            c.OMSetBlendState(self.blend.as_ref(), Some(&self.blend_factor), self.sample_mask);
            c.OMSetDepthStencilState(self.depth.as_ref(), self.stencil_ref);
            c.PSSetShaderResources(0, Some(&self.ps_srv));
            c.PSSetSamplers(0, Some(&self.ps_sampler));
            c.PSSetShader(self.ps.as_ref(), Some(&self.ps_inst));
            c.PSSetConstantBuffers(0, Some(&self.ps_cb));
            c.VSSetShader(self.vs.as_ref(), Some(&self.vs_inst));
            c.VSSetConstantBuffers(0, Some(&self.vs_cb));
            c.GSSetShader(self.gs.as_ref(), None);
            c.HSSetShader(self.hs.as_ref(), None);
            c.DSSetShader(self.ds.as_ref(), None);
            c.IASetPrimitiveTopology(self.topology);
            c.IASetIndexBuffer(self.ib.as_ref(), self.ib_format, self.ib_offset);
            c.IASetVertexBuffers(0, 1, Some(&self.vb), Some(&self.vb_stride), Some(&self.vb_offset));
            c.IASetInputLayout(self.layout.as_ref());
            c.OMSetRenderTargets(Some(&self.rtvs), self.dsv.as_ref());
        }
    }
}

/// How a back buffer's pixels are laid out, for screenshots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PixelKind {
    Rgba8,
    Bgra8,
    Rgb10a2,
    /// 16-bit float RGBA, linear (HDR or scRGB): converted to sRGB.
    Half,
}

pub(super) fn pixel_kind(f: DXGI_FORMAT) -> Option<PixelKind> {
    Some(match f {
        DXGI_FORMAT_R8G8B8A8_TYPELESS | DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => PixelKind::Rgba8,
        DXGI_FORMAT_B8G8R8A8_TYPELESS | DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB | DXGI_FORMAT_B8G8R8X8_UNORM => PixelKind::Bgra8,
        DXGI_FORMAT_R10G10B10A2_TYPELESS | DXGI_FORMAT_R10G10B10A2_UNORM => PixelKind::Rgb10a2,
        DXGI_FORMAT_R16G16B16A16_TYPELESS | DXGI_FORMAT_R16G16B16A16_FLOAT => PixelKind::Half,
        _ => return None,
    })
}

/// A typed format for resolving a typeless one.
fn typed(f: DXGI_FORMAT) -> DXGI_FORMAT {
    match f {
        DXGI_FORMAT_R8G8B8A8_TYPELESS => DXGI_FORMAT_R8G8B8A8_UNORM,
        DXGI_FORMAT_B8G8R8A8_TYPELESS => DXGI_FORMAT_B8G8R8A8_UNORM,
        DXGI_FORMAT_R10G10B10A2_TYPELESS => DXGI_FORMAT_R10G10B10A2_UNORM,
        DXGI_FORMAT_R16G16B16A16_TYPELESS => DXGI_FORMAT_R16G16B16A16_FLOAT,
        other => other,
    }
}

/// One row of pixels to RGB.
pub(super) fn to_rgb(kind: PixelKind, row: &[u8], out: &mut Vec<u8>) {
    match kind {
        PixelKind::Rgba8 => row.chunks_exact(4).for_each(|p| out.extend_from_slice(&p[..3])),
        PixelKind::Bgra8 => row.chunks_exact(4).for_each(|p| out.extend_from_slice(&[p[2], p[1], p[0]])),
        PixelKind::Rgb10a2 => row.chunks_exact(4).for_each(|p| {
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            out.extend_from_slice(&[((v & 0x3FF) >> 2) as u8, (((v >> 10) & 0x3FF) >> 2) as u8, (((v >> 20) & 0x3FF) >> 2) as u8]);
        }),
        PixelKind::Half => row.chunks_exact(8).for_each(|p| {
            for c in 0..3 {
                out.push(linear_to_srgb8(half(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]]))));
            }
        }),
    }
}

/// IEEE half float to f32.
fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let frac = (h & 0x3FF) as f32;
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn linear_to_srgb8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod shot_tests {
    use super::*;

    #[test]
    fn converts_back_buffer_pixels() {
        let mut out = Vec::new();
        to_rgb(PixelKind::Rgba8, &[1, 2, 3, 255], &mut out);
        to_rgb(PixelKind::Bgra8, &[1, 2, 3, 255], &mut out);
        // 10-bit red 1023, green 0, blue 512.
        to_rgb(PixelKind::Rgb10a2, &(1023u32 | (512 << 20)).to_le_bytes(), &mut out);
        // Half floats: 1.0 (0x3C00), 0.0, 0.5 (0x3800).
        to_rgb(PixelKind::Half, &[0x00, 0x3C, 0x00, 0x00, 0x00, 0x38, 0x00, 0x3C], &mut out);
        assert_eq!(out, vec![1, 2, 3, 3, 2, 1, 255, 0, 128, 255, 0, 188]);
        assert_eq!(pixel_kind(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB), Some(PixelKind::Bgra8));
        assert_eq!(pixel_kind(DXGI_FORMAT_D32_FLOAT), None);
    }
}


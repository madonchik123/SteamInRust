//! Drawing the overlay inside a D3D11 game.
//!
//! We make a throwaway device and swap chain on a hidden window to find
//! DXGI's swap chain method table, then point `Present` (and `Present1`,
//! `ResizeBuffers`, `ResizeBuffers1`) at our functions. Every swap chain in
//! the process shares that table, so the game's own frames come through
//! here: we draw the overlay's triangles on top of the back buffer, put the
//! game's D3D11 state back exactly as it was, and call the real `Present`.
//!
//! A game that isn't D3D11 (D3D9/12, OpenGL, Vulkan) never sends a D3D11
//! frame through, so the app keeps its window overlay for it.

use super::{Frame, TexOp, shared};
use crate::overlay::wire::VERTEX_SIZE;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
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

unsafe fn patch(vtable: *mut usize, slot: usize, ours: usize, real: &AtomicUsize) {
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
    if flags & DXGI_PRESENT_TEST.0 == 0 {
        on_present(this);
    }
    let real: PresentFn = unsafe { std::mem::transmute(REAL_PRESENT.load(Ordering::Acquire)) };
    unsafe { real(this, sync, flags) }
}

unsafe extern "system" fn present1(this: *mut c_void, sync: u32, flags: u32, params: *const c_void) -> HRESULT {
    if flags & DXGI_PRESENT_TEST.0 == 0 {
        on_present(this);
    }
    let real: Present1Fn = unsafe { std::mem::transmute(REAL_PRESENT1.load(Ordering::Acquire)) };
    unsafe { real(this, sync, flags, params) }
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
    /// The render target stores linear light (sRGB view or float), so
    /// egui's gamma-space colors are converted in the shader.
    linear_out: bool,
    size: (u32, u32),
    textures: HashMap<u64, Tex>,
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
unsafe fn compile(entry: PCSTR, target: PCSTR) -> windows::core::Result<Vec<u8>> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type CompileFn = unsafe extern "system" fn(*const c_void, usize, PCSTR, *const c_void, *mut c_void, PCSTR, PCSTR, u32, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
    unsafe {
        let module = LoadLibraryW(w!("d3dcompiler_47.dll"))?;
        let Some(f) = GetProcAddress(module, s!("D3DCompile")) else { return Err(windows::core::Error::from_thread()) };
        let f: CompileFn = std::mem::transmute(f);
        let (mut code, mut errors) = (std::ptr::null_mut(), std::ptr::null_mut());
        let hr = f(SHADER.as_ptr().cast(), SHADER.len(), PCSTR::null(), std::ptr::null(), std::ptr::null_mut(), entry, target, 0, 0, &mut code, &mut errors);
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

fn out<T>(slot: Option<T>) -> windows::core::Result<T> {
    slot.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))
}

impl Renderer {
    unsafe fn new(sc: &IDXGISwapChain, raw: usize) -> windows::core::Result<Self> {
        unsafe {
            let device: ID3D11Device = sc.GetDevice()?;
            let ctx = device.GetImmediateContext()?;
            let desc = sc.GetDesc()?;
            let vs_code = compile(s!("vs"), s!("vs_4_0"))?;
            let ps_code = compile(s!("ps"), s!("ps_4_0"))?;
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
                linear_out: false,
                size: (desc.BufferDesc.Width, desc.BufferDesc.Height),
                textures: HashMap::new(),
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

    unsafe fn draw(&mut self, sc: &IDXGISwapChain, frame: &Frame) -> windows::core::Result<()> {
        unsafe {
            self.ensure_target(sc)?;
            let vbytes: usize = frame.meshes.iter().map(|m| m.vertices.len()).sum();
            let ibytes: usize = frame.meshes.iter().map(|m| m.indices.len() * 4).sum();
            if vbytes == 0 || ibytes == 0 {
                return Ok(());
            }
            let vb = Self::buffer(&self.device, &mut self.vb, vbytes, D3D11_BIND_VERTEX_BUFFER)?;
            let ib = Self::buffer(&self.device, &mut self.ib, ibytes, D3D11_BIND_INDEX_BUFFER)?;
            self.upload(&vb, &mut frame.meshes.iter().map(|m| m.vertices.as_slice()))?;
            let idx: Vec<&[u8]> = frame.meshes.iter().map(|m| std::slice::from_raw_parts(m.indices.as_ptr() as *const u8, m.indices.len() * 4)).collect();
            self.upload(&ib, &mut idx.into_iter())?;
            let ppp = frame.pixels_per_point.max(0.1);
            let (w, h) = (self.size.0 as f32, self.size.1 as f32);
            let constants = [w / ppp, h / ppp, if self.linear_out { 1.0 } else { 0.0 }, 0.0f32];
            let cb_bytes = std::slice::from_raw_parts(constants.as_ptr() as *const u8, 16);
            self.upload(&self.constants, &mut std::iter::once(cb_bytes))?;

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

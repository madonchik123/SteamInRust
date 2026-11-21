//! Drawing the overlay inside a D3D12 game.
//!
//! D3D12 swap chains come through the same `Present` hook as D3D11 ones (see
//! `d3d11.rs`). What differs is everything after it:
//!
//! - D3D12 has no "current context": work goes on a command queue, and a swap
//!   chain doesn't say which one it presents from. We watch
//!   `ExecuteCommandLists` to learn the game's direct queues, then pick the
//!   one the swap chain holds.
//! - Our commands run after the game's on that queue, right before `Present`,
//!   with their own lists, so the game's state isn't touched at all.
//! - Nothing may be freed or reused while the GPU still reads it, so each
//!   frame has its own allocator and buffers, tracked with a fence. Every
//!   wait has a timeout: if the GPU is busy we skip drawing a frame instead
//!   of holding the game up.

use super::d3d11::{out, pixel_kind, stats, to_rgb};
use super::wire::VERTEX_SIZE;
use super::{Frame, TexOp, shared};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, HWND, RECT, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{Error, HRESULT, Interface, Result, s, w};

/// ID3D12CommandQueue: IUnknown 0-2, ID3D12Object 3-6, GetDevice 7,
/// UpdateTileMappings 8, CopyTileMappings 9.
const SLOT_EXECUTE: usize = 10;
/// Shader-visible texture slots.
const SRV_SLOTS: u32 = 2048;
/// Longest we hold the game's frame up waiting for the GPU.
const WAIT_FRAME_MS: u32 = 8;
const WAIT_IDLE_MS: u32 = 2000;

type ExecuteFn = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);

static REAL_EXECUTE: AtomicUsize = AtomicUsize::new(0);
/// The direct queue that last ran commands.
static LAST_DIRECT: AtomicUsize = AtomicUsize::new(0);
/// The last queue seen that isn't a direct one (copy, compute).
static LAST_OTHER: AtomicUsize = AtomicUsize::new(0);
/// Every direct queue seen.
static DIRECT: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// The last swap chain found not to be D3D12, and the last that was.
static NOT_12: AtomicUsize = AtomicUsize::new(0);
static IS_12: AtomicUsize = AtomicUsize::new(0);

/// Runs for every submission the game makes, so it stays very cheap.
unsafe extern "system" fn execute(this: *mut c_void, count: u32, lists: *const *mut c_void) {
    let raw = this as usize;
    if raw != LAST_DIRECT.load(Ordering::Relaxed) && raw != LAST_OTHER.load(Ordering::Relaxed) {
        let direct = unsafe { ID3D12CommandQueue::from_raw_borrowed(&this) }.is_some_and(|q| unsafe { q.GetDesc() }.Type == D3D12_COMMAND_LIST_TYPE_DIRECT);
        if direct {
            LAST_DIRECT.store(raw, Ordering::Relaxed);
            if let Ok(mut known) = DIRECT.lock() {
                if !known.contains(&raw) {
                    known.push(raw);
                }
            }
        } else {
            LAST_OTHER.store(raw, Ordering::Relaxed);
        }
    }
    let real: ExecuteFn = unsafe { std::mem::transmute(REAL_EXECUTE.load(Ordering::Acquire)) };
    unsafe { real(this, count, lists) }
}

/// Watch the device's queues (their method table is shared).
unsafe fn hook_queues(device: &ID3D12Device) -> Result<()> {
    if REAL_EXECUTE.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    unsafe {
        let desc = D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() };
        let queue: ID3D12CommandQueue = device.CreateCommandQueue(&desc)?;
        let vtable = *(queue.as_raw() as *mut *mut usize);
        super::d3d11::patch(vtable, SLOT_EXECUTE, execute as *const () as usize, &REAL_EXECUTE);
    }
    if REAL_EXECUTE.load(Ordering::Acquire) == 0 {
        return Err(Error::new(E_FAIL, "couldn't watch the command queues"));
    }
    crate::log!("overlay: watching the game's D3D12 command queues");
    Ok(())
}

/// The queue the swap chain presents from. DXGI keeps a pointer to it inside
/// the swap chain, so look for one of the game's queues there. A pointer
/// that matches a queue we saw run commands can't be a false positive that
/// matters; if none is found, the last direct queue is the usual answer.
unsafe fn find_queue(swapchain: usize, device: &ID3D12Device) -> Option<ID3D12CommandQueue> {
    use windows::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery};
    let known = DIRECT.lock().ok()?.clone();
    if known.is_empty() {
        return None;
    }
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let got = unsafe { VirtualQuery(Some(swapchain as *const c_void), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
    let mut found = None;
    if got != 0 && info.State == MEM_COMMIT && info.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) == 0 {
        let end = info.BaseAddress as usize + info.RegionSize;
        let len = end.saturating_sub(swapchain).min(0x1000) / size_of::<usize>();
        let words = unsafe { std::slice::from_raw_parts(swapchain as *const usize, len) };
        found = words.iter().copied().find(|w| known.contains(w));
    }
    let matched = found.is_some();
    // Several queues and none named by the swap chain: drawing on the wrong
    // one could corrupt the game's frame, so leave it to the window overlay.
    if !matched && known.len() > 1 {
        static TOLD: std::sync::Once = std::sync::Once::new();
        TOLD.call_once(|| crate::log!("overlay: can't tell which of {} D3D12 queues presents, not drawing", known.len()));
        return None;
    }
    let raw = found.unwrap_or_else(|| LAST_DIRECT.load(Ordering::Relaxed));
    let p = raw as *mut c_void;
    let queue = unsafe { ID3D12CommandQueue::from_raw_borrowed(&p) }?.clone();
    // A queue of another device can't touch these back buffers.
    let mut owner: Option<ID3D12Device> = None;
    unsafe { queue.GetDevice(&mut owner) }.ok()?;
    let same = owner?.as_raw() == device.as_raw();
    if !same {
        return None;
    }
    crate::log!("overlay: drawing on the game's direct queue ({} known, {})", known.len(), if matched { "the swap chain's" } else { "the last one used" });
    Some(queue)
}

struct UploadBuffer {
    resource: ID3D12Resource,
    mapped: *mut u8,
    capacity: usize,
}

struct Tex {
    resource: ID3D12Resource,
    slot: u32,
    size: [u32; 2],
    linear: bool,
}

/// What one submitted frame holds until the GPU is done with it.
struct FrameContext {
    allocator: ID3D12CommandAllocator,
    fence_value: u64,
    vb: Option<UploadBuffer>,
    ib: Option<UploadBuffer>,
    /// The app frame whose triangles are in `vb` and `ib`.
    uploaded: Option<Arc<Frame>>,
    /// Texture upload buffers used by this frame.
    staging: Vec<ID3D12Resource>,
    /// Textures the app freed; they go when this frame is done.
    dead: Vec<Tex>,
}

struct Renderer {
    swapchain: usize,
    hwnd: HWND,
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    fence: ID3D12Fence,
    fence_value: u64,
    event: HANDLE,
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    format: DXGI_FORMAT,
    list: ID3D12GraphicsCommandList,
    rtv_heap: ID3D12DescriptorHeap,
    rtv_step: usize,
    srv_heap: ID3D12DescriptorHeap,
    srv_step: u64,
    free_slots: Vec<u32>,
    back_buffers: Vec<ID3D12Resource>,
    frames: Vec<FrameContext>,
    next_frame: usize,
    textures: HashMap<u64, Tex>,
    /// Texture changes that couldn't be applied yet (GPU busy).
    pending: Vec<TexOp>,
    size: (u32, u32),
    linear_out: bool,
    fps: super::Fps,
    last_present: Instant,
}

// COM pointers used only under RENDERER's lock, from whatever thread presents.
unsafe impl Send for Renderer {}

static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
static BACKOFF: Mutex<Backoff> = Mutex::new(Backoff { fails: 0, last: None });

/// Failures in a row before the overlay stops drawing in this game.
const MAX_FAILS: u32 = 3;
/// Failures further apart than this don't count as "in a row".
const FAILS_FORGOTTEN: Duration = Duration::from_secs(60);

/// Rebuilding the renderer compiles shaders, which a game would feel if it
/// happened every frame: wait longer after each failure, then give up.
struct Backoff {
    fails: u32,
    last: Option<Instant>,
}

impl Backoff {
    fn due(&self, now: Instant) -> bool {
        self.last.is_none_or(|last| now >= last + Duration::from_secs(2) * self.fails)
    }

    /// One more failure. True when it's time to give up.
    fn failed(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|last| now.duration_since(last) > FAILS_FORGOTTEN) {
            self.fails = 0;
        }
        self.fails += 1;
        self.last = Some(now);
        self.fails >= MAX_FAILS
    }
}

/// Handle a frame if `this` is a D3D12 swap chain. False means it isn't.
pub fn on_present(sc: &IDXGISwapChain, this: usize) -> bool {
    if NOT_12.load(Ordering::Relaxed) == this {
        return false;
    }
    if IS_12.load(Ordering::Relaxed) == this && super::gave_up() {
        return true;
    }
    if IS_12.load(Ordering::Relaxed) != this {
        match unsafe { sc.GetDevice::<ID3D12Device>() } {
            Ok(device) => {
                IS_12.store(this, Ordering::Relaxed);
                if let Err(e) = unsafe { hook_queues(&device) } {
                    crate::log!("overlay: D3D12 not hooked: {e}");
                }
            }
            Err(_) => {
                NOT_12.store(this, Ordering::Relaxed);
                return false;
            }
        }
    }
    if let Ok(mut guard) = RENDERER.lock() {
        if guard.is_none() && !BACKOFF.lock().is_ok_and(|b| b.due(Instant::now())) {
            return true;
        }
        if let Err(e) = unsafe { frame(&mut guard, sc, this) } {
            crate::log!("overlay: D3D12 frame skipped: {e}");
            // Something is wrong with what we hold (device lost, swap chain
            // gone): start over on a later frame rather than keep failing.
            *guard = None;
            if BACKOFF.lock().map_or(true, |mut b| b.failed(Instant::now())) {
                crate::log!("overlay: giving up on D3D12 in this game; the game carries on");
                super::give_up();
            }
        }
    }
    true
}

/// The back buffers can't be resized while we hold them.
pub fn before_resize(this: usize) {
    if let Ok(mut guard) = RENDERER.lock() {
        if let Some(r) = guard.as_mut().filter(|r| r.swapchain == this) {
            unsafe { r.wait_idle() };
            r.back_buffers.clear();
        }
    }
}

unsafe fn frame(guard: &mut Option<Renderer>, sc: &IDXGISwapChain, this: usize) -> Result<()> {
    if let Some(r) = guard.as_ref() {
        if r.swapchain != this {
            if r.last_present.elapsed().as_secs_f32() < 1.0 {
                return Ok(());
            }
            crate::log!("overlay: following a new D3D12 swap chain");
            *guard = None;
        }
    }
    if guard.is_none() {
        let device: ID3D12Device = unsafe { sc.GetDevice() }?;
        // Not known yet: the game hasn't submitted since we started watching.
        let Some(queue) = (unsafe { find_queue(this, &device) }) else { return Ok(()) };
        let r = unsafe { Renderer::new(sc, this, device, queue) }?;
        crate::log!("overlay: drawing into the game's D3D12 swap chain ({}x{}, {:?})", r.size.0, r.size.1, r.format);
        super::input::attach(r.hwnd);
        super::hooked(r.size);
        *guard = Some(r);
    }
    let r = guard.as_mut().expect("just set");
    r.last_present = Instant::now();
    let desc = unsafe { sc.GetDesc() }?;
    let size = (desc.BufferDesc.Width, desc.BufferDesc.Height);
    if size != r.size {
        unsafe { r.wait_idle() };
        r.back_buffers.clear();
        r.size = size;
        super::resized(size);
    }
    if desc.BufferDesc.Format != r.format {
        unsafe { r.wait_idle() };
        r.back_buffers.clear();
        unsafe { r.set_format(desc.BufferDesc.Format) }?;
    }
    if desc.OutputWindow != r.hwnd {
        r.hwnd = desc.OutputWindow;
        super::input::attach(r.hwnd);
    }
    r.fps.tick();
    if r.back_buffers.is_empty() {
        unsafe { r.get_back_buffers(sc, desc.BufferCount) }?;
    }
    let index = unsafe { sc.cast::<IDXGISwapChain3>()?.GetCurrentBackBufferIndex() } as usize;
    if index >= r.back_buffers.len() {
        return Ok(());
    }
    if super::SHOT.swap(false, Ordering::Relaxed) {
        match unsafe { r.capture(index) } {
            Ok((rgb, w, h)) => crate::screenshots::save_frame(rgb, w, h),
            Err(e) => {
                crate::log!("overlay: screenshot copy failed: {e}");
                crate::screenshots::capture_window();
            }
        }
    }
    let (ops, app_frame) = shared::take_for_present();
    r.pending.extend(ops);
    let app_frame = app_frame.filter(|f| !f.meshes.is_empty());
    if r.pending.is_empty() && app_frame.is_none() {
        return Ok(());
    }
    unsafe { r.submit(index, app_frame.as_ref()) }
}

const SHADER: &str = r#"
cbuffer C : register(b0) { float2 screen; float linear_out; float nearest; };
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
SamplerState linear_sampler : register(s0);
SamplerState point_sampler : register(s1);
float3 to_linear(float3 c) { c = max(c, 0.0); return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }
float4 ps(PI i) : SV_Target {
    float4 t = nearest > 0.5 ? t0.Sample(point_sampler, i.uv) : t0.Sample(linear_sampler, i.uv);
    float4 c = i.col * t;
    if (linear_out > 0.5) c.rgb = to_linear(c.rgb);
    return c;
}
"#;

fn heap(kind: D3D12_HEAP_TYPE) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES { Type: kind, CreationNodeMask: 1, VisibleNodeMask: 1, ..Default::default() }
}

fn buffer_desc(size: u64) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        ..Default::default()
    }
}

/// A resource transition. The barrier borrows the resource without holding
/// a reference, which is how these structs are meant to be used.
fn transition(resource: &ID3D12Resource, before: D3D12_RESOURCE_STATES, after: D3D12_RESOURCE_STATES) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: unsafe { std::mem::transmute_copy(resource) },
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

fn copy_subresource(resource: &ID3D12Resource) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: unsafe { std::mem::transmute_copy(resource) },
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
    }
}

fn copy_footprint(resource: &ID3D12Resource, format: DXGI_FORMAT, width: u32, height: u32, pitch: u32) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: unsafe { std::mem::transmute_copy(resource) },
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: 0,
                Footprint: D3D12_SUBRESOURCE_FOOTPRINT { Format: format, Width: width, Height: height, Depth: 1, RowPitch: pitch },
            },
        },
    }
}

fn aligned_pitch(bytes: u32) -> u32 {
    let a = D3D12_TEXTURE_DATA_PITCH_ALIGNMENT;
    bytes.div_ceil(a) * a
}

unsafe fn root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature> {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    type SerializeFn = unsafe extern "system" fn(*const D3D12_ROOT_SIGNATURE_DESC, D3D_ROOT_SIGNATURE_VERSION, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
    unsafe {
        let range = D3D12_DESCRIPTOR_RANGE { RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV, NumDescriptors: 1, BaseShaderRegister: 0, RegisterSpace: 0, OffsetInDescriptorsFromTableStart: 0 };
        let params = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                Anonymous: D3D12_ROOT_PARAMETER_0 { Constants: D3D12_ROOT_CONSTANTS { ShaderRegister: 0, RegisterSpace: 0, Num32BitValues: 4 } },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: &range } },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ];
        let sampler = |filter, register| D3D12_STATIC_SAMPLER_DESC {
            Filter: filter,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
            MaxLOD: f32::MAX,
            ShaderRegister: register,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            ..Default::default()
        };
        let samplers = [sampler(D3D12_FILTER_MIN_MAG_MIP_LINEAR, 0), sampler(D3D12_FILTER_MIN_MAG_MIP_POINT, 1)];
        let desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: params.len() as u32,
            pParameters: params.as_ptr(),
            NumStaticSamplers: samplers.len() as u32,
            pStaticSamplers: samplers.as_ptr(),
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };
        // Looked up, so this DLL doesn't depend on d3d12.dll in other games.
        let module = GetModuleHandleW(w!("d3d12.dll"))?;
        let Some(f) = GetProcAddress(module, s!("D3D12SerializeRootSignature")) else { return Err(Error::from_thread()) };
        let f: SerializeFn = std::mem::transmute(f);
        let (mut blob, mut errors) = (std::ptr::null_mut(), std::ptr::null_mut());
        let hr = f(&desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut blob, &mut errors);
        if !errors.is_null() {
            let e = ID3DBlob::from_raw(errors);
            let text = std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize());
            crate::log!("overlay: root signature: {}", String::from_utf8_lossy(text));
        }
        hr.ok()?;
        let blob = ID3DBlob::from_raw(blob);
        device.CreateRootSignature(0, std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()))
    }
}

unsafe fn pipeline(device: &ID3D12Device, root: &ID3D12RootSignature, format: DXGI_FORMAT) -> Result<ID3D12PipelineState> {
    unsafe {
        let vs = super::d3d11::compile_source(SHADER, s!("vs"), s!("vs_5_0"))?;
        let ps = super::d3d11::compile_source(SHADER, s!("ps"), s!("ps_5_0"))?;
        let elements = [
            D3D12_INPUT_ELEMENT_DESC { SemanticName: s!("POSITION"), SemanticIndex: 0, Format: DXGI_FORMAT_R32G32_FLOAT, InputSlot: 0, AlignedByteOffset: 0, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
            D3D12_INPUT_ELEMENT_DESC { SemanticName: s!("TEXCOORD"), SemanticIndex: 0, Format: DXGI_FORMAT_R32G32_FLOAT, InputSlot: 0, AlignedByteOffset: 8, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
            D3D12_INPUT_ELEMENT_DESC { SemanticName: s!("COLOR"), SemanticIndex: 0, Format: DXGI_FORMAT_R8G8B8A8_UNORM, InputSlot: 0, AlignedByteOffset: 16, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, InstanceDataStepRate: 0 },
        ];
        let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(root),
            VS: D3D12_SHADER_BYTECODE { pShaderBytecode: vs.as_ptr().cast(), BytecodeLength: vs.len() },
            PS: D3D12_SHADER_BYTECODE { pShaderBytecode: ps.as_ptr().cast(), BytecodeLength: ps.len() },
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC { FillMode: D3D12_FILL_MODE_SOLID, CullMode: D3D12_CULL_MODE_NONE, DepthClipEnable: true.into(), ..Default::default() },
            InputLayout: D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elements.as_ptr(), NumElements: elements.len() as u32 },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };
        desc.RTVFormats[0] = format;
        // Premultiplied alpha, like egui's own backends.
        desc.BlendState.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D12_BLEND_ONE,
            DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_INV_DEST_ALPHA,
            DestBlendAlpha: D3D12_BLEND_ONE,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: D3D12_LOGIC_OP_NOOP,
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
            ..Default::default()
        };
        device.CreateGraphicsPipelineState(&desc)
    }
}

impl Renderer {
    unsafe fn new(sc: &IDXGISwapChain, raw: usize, device: ID3D12Device, queue: ID3D12CommandQueue) -> Result<Self> {
        unsafe {
            let desc = sc.GetDesc()?;
            let format = desc.BufferDesc.Format;
            let root = root_signature(&device)?;
            let pso = pipeline(&device, &root, format)?;
            let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
            let event = CreateEventW(None, false, false, None)?;
            let buffers = desc.BufferCount.max(1);
            let mut frames = Vec::new();
            for _ in 0..buffers + 1 {
                frames.push(FrameContext {
                    allocator: device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?,
                    fence_value: 0,
                    vb: None,
                    ib: None,
                    uploaded: None,
                    staging: Vec::new(),
                    dead: Vec::new(),
                });
            }
            let list: ID3D12GraphicsCommandList = device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &frames[0].allocator, &pso)?;
            list.Close()?;
            // Enough views for any swap chain (they hold at most 16 buffers).
            let rtv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC { Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV, NumDescriptors: 16, ..Default::default() })?;
            let srv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: SRV_SLOTS,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })?;
            Ok(Renderer {
                swapchain: raw,
                hwnd: desc.OutputWindow,
                rtv_step: device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) as usize,
                srv_step: device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) as u64,
                device,
                queue,
                fence,
                fence_value: 0,
                event,
                root,
                pso,
                format,
                list,
                rtv_heap,
                srv_heap,
                free_slots: (0..SRV_SLOTS).rev().collect(),
                back_buffers: Vec::new(),
                frames,
                next_frame: 0,
                textures: HashMap::new(),
                pending: Vec::new(),
                size: (desc.BufferDesc.Width, desc.BufferDesc.Height),
                linear_out: is_linear(format),
                fps: super::Fps::new(),
                last_present: Instant::now(),
            })
        }
    }

    unsafe fn set_format(&mut self, format: DXGI_FORMAT) -> Result<()> {
        self.pso = unsafe { pipeline(&self.device, &self.root, format) }?;
        self.format = format;
        self.linear_out = is_linear(format);
        Ok(())
    }

    unsafe fn get_back_buffers(&mut self, sc: &IDXGISwapChain, count: u32) -> Result<()> {
        unsafe {
            let start = self.rtv_heap.GetCPUDescriptorHandleForHeapStart();
            let mut buffers = Vec::new();
            for i in 0..count.min(16) {
                let b: ID3D12Resource = sc.GetBuffer(i)?;
                self.device.CreateRenderTargetView(&b, None, D3D12_CPU_DESCRIPTOR_HANDLE { ptr: start.ptr + i as usize * self.rtv_step });
                buffers.push(b);
            }
            self.back_buffers = buffers;
            Ok(())
        }
    }

    /// Wait up to `ms` for the GPU to reach `value`.
    unsafe fn wait(&self, value: u64, ms: u32) -> bool {
        unsafe {
            if self.fence.GetCompletedValue() >= value {
                return true;
            }
            if self.fence.SetEventOnCompletion(value, self.event).is_err() {
                return false;
            }
            WaitForSingleObject(self.event, ms) == WAIT_OBJECT_0 || self.fence.GetCompletedValue() >= value
        }
    }

    unsafe fn wait_idle(&self) -> bool {
        unsafe { self.wait(self.fence_value, WAIT_IDLE_MS) }
    }

    /// Run the list we just recorded, and mark it with the next fence value.
    unsafe fn execute(&mut self) -> Result<u64> {
        unsafe {
            self.list.Close()?;
            let list: ID3D12CommandList = self.list.cast()?;
            self.queue.ExecuteCommandLists(&[Some(list)]);
            self.fence_value += 1;
            self.queue.Signal(&self.fence, self.fence_value)?;
            Ok(self.fence_value)
        }
    }

    /// A frame context the GPU is finished with, its old resources released.
    /// `None` when all are still busy after a short wait.
    unsafe fn acquire(&mut self) -> Result<Option<usize>> {
        let at = self.next_frame;
        let value = self.frames[at].fence_value;
        if !unsafe { self.wait(value, WAIT_FRAME_MS) } {
            return Ok(None);
        }
        self.next_frame = (at + 1) % self.frames.len();
        let f = &mut self.frames[at];
        f.staging.clear();
        for tex in f.dead.drain(..) {
            self.free_slots.push(tex.slot);
        }
        unsafe {
            f.allocator.Reset()?;
            self.list.Reset(&f.allocator, &self.pso)?;
        }
        Ok(Some(at))
    }

    unsafe fn upload_buffer(device: &ID3D12Device, bytes: usize) -> Result<UploadBuffer> {
        unsafe {
            let capacity = bytes.next_power_of_two().max(64 * 1024);
            let mut res: Option<ID3D12Resource> = None;
            device.CreateCommittedResource(&heap(D3D12_HEAP_TYPE_UPLOAD), D3D12_HEAP_FLAG_NONE, &buffer_desc(capacity as u64), D3D12_RESOURCE_STATE_GENERIC_READ, None, &mut res)?;
            let resource = out(res)?;
            let mut mapped = std::ptr::null_mut();
            // No range to read: the CPU only writes these.
            resource.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped))?;
            Ok(UploadBuffer { resource, mapped: mapped as *mut u8, capacity })
        }
    }

    /// Record one texture change. Replaced textures wait in `frame`'s list
    /// until the GPU is done with them.
    unsafe fn apply(&mut self, at: usize, op: TexOp) -> Result<()> {
        match op {
            TexOp::Free(id) => {
                if let Some(t) = self.textures.remove(&id) {
                    self.frames[at].dead.push(t);
                }
                Ok(())
            }
            TexOp::Set(t) => unsafe {
                stats::TEX_OPS.fetch_add(1, Ordering::Relaxed);
                stats::TEX_BYTES.fetch_add(t.rgba.len() as u64, Ordering::Relaxed);
                let [w, h] = t.size;
                if w == 0 || h == 0 {
                    return Ok(());
                }
                let (x, y, fresh) = match t.pos {
                    Some([x, y]) => {
                        let Some(tex) = self.textures.get_mut(&t.id) else { return Ok(()) };
                        if x + w > tex.size[0] || y + h > tex.size[1] {
                            return Ok(());
                        }
                        tex.linear = t.linear;
                        (x, y, false)
                    }
                    None => {
                        let Some(slot) = self.free_slots.pop() else {
                            crate::log!("overlay: out of D3D12 texture slots");
                            return Ok(());
                        };
                        let desc = D3D12_RESOURCE_DESC {
                            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                            Width: w as u64,
                            Height: h,
                            DepthOrArraySize: 1,
                            MipLevels: 1,
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                            ..Default::default()
                        };
                        let mut res: Option<ID3D12Resource> = None;
                        if let Err(e) = self.device.CreateCommittedResource(&heap(D3D12_HEAP_TYPE_DEFAULT), D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COPY_DEST, None, &mut res) {
                            self.free_slots.push(slot);
                            return Err(e);
                        }
                        let resource = out(res)?;
                        let view = D3D12_SHADER_RESOURCE_VIEW_DESC {
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() } },
                        };
                        let cpu = self.srv_heap.GetCPUDescriptorHandleForHeapStart();
                        self.device.CreateShaderResourceView(&resource, Some(&view), D3D12_CPU_DESCRIPTOR_HANDLE { ptr: cpu.ptr + slot as usize * self.srv_step as usize });
                        if let Some(old) = self.textures.insert(t.id, Tex { resource, slot, size: t.size, linear: t.linear }) {
                            self.frames[at].dead.push(old);
                        }
                        (0, 0, true)
                    }
                };
                // The pixels, row by row into a buffer the GPU copies from.
                let pitch = aligned_pitch(w * 4);
                let bytes = pitch as usize * h as usize;
                let mut res: Option<ID3D12Resource> = None;
                self.device.CreateCommittedResource(&heap(D3D12_HEAP_TYPE_UPLOAD), D3D12_HEAP_FLAG_NONE, &buffer_desc(bytes as u64), D3D12_RESOURCE_STATE_GENERIC_READ, None, &mut res)?;
                let staging = out(res)?;
                let mut mapped = std::ptr::null_mut();
                staging.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped))?;
                for row in 0..h as usize {
                    let from = t.rgba.as_ptr().add(row * w as usize * 4);
                    std::ptr::copy_nonoverlapping(from, (mapped as *mut u8).add(row * pitch as usize), w as usize * 4);
                }
                staging.Unmap(0, None);
                let tex = &self.textures[&t.id];
                if !fresh {
                    self.list.ResourceBarrier(&[transition(&tex.resource, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_COPY_DEST)]);
                }
                let dst = copy_subresource(&tex.resource);
                let src = copy_footprint(&staging, DXGI_FORMAT_R8G8B8A8_UNORM, w, h, pitch);
                self.list.CopyTextureRegion(&dst, x, y, 0, &src, None);
                self.list.ResourceBarrier(&[transition(&tex.resource, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
                self.frames[at].staging.push(staging);
                Ok(())
            },
        }
    }

    /// Texture changes, then the overlay on the back buffer, ahead of Present.
    unsafe fn submit(&mut self, index: usize, app_frame: Option<&Arc<Frame>>) -> Result<()> {
        unsafe {
            // GPU busy: keep the texture changes for the next frame.
            let Some(at) = self.acquire()? else { return Ok(()) };
            for op in std::mem::take(&mut self.pending) {
                self.apply(at, op)?;
            }
            if let Some(f) = app_frame {
                self.record_draw(at, index, f)?;
            }
            let value = self.execute()?;
            self.frames[at].fence_value = value;
            Ok(())
        }
    }

    unsafe fn record_draw(&mut self, at: usize, index: usize, frame: &Arc<Frame>) -> Result<()> {
        unsafe {
            let vbytes: usize = frame.meshes.iter().map(|m| m.vertices.len()).sum();
            let ibytes: usize = frame.meshes.iter().map(|m| m.indices.len() * 4).sum();
            if vbytes == 0 || ibytes == 0 {
                return Ok(());
            }
            let ctx = &mut self.frames[at];
            let mut grew = false;
            if ctx.vb.as_ref().is_none_or(|b| b.capacity < vbytes) {
                ctx.vb = Some(Self::upload_buffer(&self.device, vbytes)?);
                grew = true;
            }
            if ctx.ib.as_ref().is_none_or(|b| b.capacity < ibytes) {
                ctx.ib = Some(Self::upload_buffer(&self.device, ibytes)?);
                grew = true;
            }
            let (vb, ib) = (ctx.vb.as_ref().expect("just made"), ctx.ib.as_ref().expect("just made"));
            // Each context has its own copy; refill it only for a new frame.
            if grew || !ctx.uploaded.as_ref().is_some_and(|f| Arc::ptr_eq(f, frame)) {
                let (mut v, mut i) = (vb.mapped, ib.mapped);
                for m in &frame.meshes {
                    std::ptr::copy_nonoverlapping(m.vertices.as_ptr(), v, m.vertices.len());
                    v = v.add(m.vertices.len());
                    std::ptr::copy_nonoverlapping(m.indices.as_ptr() as *const u8, i, m.indices.len() * 4);
                    i = i.add(m.indices.len() * 4);
                }
                ctx.uploaded = Some(frame.clone());
                stats::UPLOADS.fetch_add(1, Ordering::Relaxed);
            }
            let vb_view = D3D12_VERTEX_BUFFER_VIEW { BufferLocation: vb.resource.GetGPUVirtualAddress(), SizeInBytes: vbytes as u32, StrideInBytes: VERTEX_SIZE as u32 };
            let ib_view = D3D12_INDEX_BUFFER_VIEW { BufferLocation: ib.resource.GetGPUVirtualAddress(), SizeInBytes: ibytes as u32, Format: DXGI_FORMAT_R32_UINT };
            let back = &self.back_buffers[index];
            let l = &self.list;
            l.ResourceBarrier(&[transition(back, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET)]);
            let start = self.rtv_heap.GetCPUDescriptorHandleForHeapStart();
            let rtv = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: start.ptr + index * self.rtv_step };
            l.OMSetRenderTargets(1, Some(&rtv), false, None);
            let (w, h) = (self.size.0 as f32, self.size.1 as f32);
            l.RSSetViewports(&[D3D12_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w, Height: h, MinDepth: 0.0, MaxDepth: 1.0 }]);
            l.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            l.SetGraphicsRootSignature(&self.root);
            l.SetPipelineState(&self.pso);
            l.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            l.IASetVertexBuffers(0, Some(&[vb_view]));
            l.IASetIndexBuffer(Some(&ib_view));
            let ppp = frame.pixels_per_point.max(0.1);
            let constants = [w / ppp, h / ppp, if self.linear_out { 1.0 } else { 0.0 }, 0.0f32];
            l.SetGraphicsRoot32BitConstants(0, 4, constants.as_ptr().cast(), 0);
            let gpu = self.srv_heap.GetGPUDescriptorHandleForHeapStart();
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
                    l.RSSetScissorRects(&[clip]);
                    l.SetGraphicsRootDescriptorTable(1, D3D12_GPU_DESCRIPTOR_HANDLE { ptr: gpu.ptr + tex.slot as u64 * self.srv_step });
                    let nearest = if tex.linear { 0.0f32 } else { 1.0 };
                    l.SetGraphicsRoot32BitConstants(0, 1, (&nearest as *const f32).cast(), 3);
                    l.DrawIndexedInstanced(count, 1, first_index, base_vertex, 0);
                }
                first_index += count;
                base_vertex += verts;
            }
            l.ResourceBarrier(&[transition(back, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PRESENT)]);
            Ok(())
        }
    }

    /// The game's finished frame as RGB, before the overlay is drawn on it.
    unsafe fn capture(&mut self, index: usize) -> Result<(Vec<u8>, u32, u32)> {
        unsafe {
            let back = self.back_buffers[index].clone();
            let desc = back.GetDesc();
            let kind = pixel_kind(desc.Format).ok_or_else(|| Error::new(E_FAIL, "unsupported back buffer format"))?;
            let (w, h) = (desc.Width as u32, desc.Height);
            let bpp = if kind == super::d3d11::PixelKind::Half { 8 } else { 4 };
            let pitch = aligned_pitch(w * bpp);
            let mut res: Option<ID3D12Resource> = None;
            self.device.CreateCommittedResource(&heap(D3D12_HEAP_TYPE_READBACK), D3D12_HEAP_FLAG_NONE, &buffer_desc(pitch as u64 * h as u64), D3D12_RESOURCE_STATE_COPY_DEST, None, &mut res)?;
            let readback = out(res)?;
            // A screenshot may wait for its frame; drawing never does.
            if !self.wait_idle() {
                return Err(Error::new(E_FAIL, "GPU busy"));
            }
            let Some(at) = self.acquire()? else { return Err(Error::new(E_FAIL, "GPU busy")) };
            self.list.ResourceBarrier(&[transition(&back, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_COPY_SOURCE)]);
            let dst = copy_footprint(&readback, desc.Format, w, h, pitch);
            let src = copy_subresource(&back);
            self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            self.list.ResourceBarrier(&[transition(&back, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_PRESENT)]);
            let value = self.execute()?;
            self.frames[at].fence_value = value;
            if !self.wait(value, WAIT_IDLE_MS) {
                // The GPU may still write to it: keep it alive with the frame.
                self.frames[at].staging.push(readback);
                return Err(Error::new(E_FAIL, "GPU busy"));
            }
            let mut mapped = std::ptr::null_mut();
            readback.Map(0, Some(&D3D12_RANGE { Begin: 0, End: pitch as usize * h as usize }), Some(&mut mapped))?;
            let mut rgb = Vec::with_capacity((w * h * 3) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts((mapped as *const u8).add(y * pitch as usize), (w * bpp) as usize);
                to_rgb(kind, row, &mut rgb);
            }
            readback.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }));
            Ok((rgb, w, h))
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Freeing what the GPU still uses would remove the game's device, so
        // if it doesn't finish in time, leak our objects instead.
        if !unsafe { self.wait_idle() } {
            crate::log!("overlay: GPU didn't go idle, leaving the overlay's D3D12 objects alive");
            std::mem::forget(std::mem::take(&mut self.frames));
            std::mem::forget(std::mem::take(&mut self.textures));
            std::mem::forget(self.list.clone());
            std::mem::forget(self.pso.clone());
            std::mem::forget(self.root.clone());
            std::mem::forget(self.srv_heap.clone());
            std::mem::forget(self.rtv_heap.clone());
        }
        let _ = unsafe { CloseHandle(self.event) };
    }
}

/// The target stores linear light, so egui's gamma colors need converting.
fn is_linear(format: DXGI_FORMAT) -> bool {
    format == DXGI_FORMAT_R16G16B16A16_FLOAT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_then_give_up() {
        let t = Instant::now();
        let mut b = Backoff { fails: 0, last: None };
        assert!(b.due(t));
        assert!(!b.failed(t));
        assert!(!b.due(t + Duration::from_secs(1)));
        assert!(b.due(t + Duration::from_secs(2)));
        assert!(!b.failed(t + Duration::from_secs(2)));
        // The wait grows with each failure.
        assert!(!b.due(t + Duration::from_secs(5)));
        assert!(b.due(t + Duration::from_secs(6)));
        assert!(b.failed(t + Duration::from_secs(6)), "third in a row gives up");

        // Failures long apart never add up.
        let mut b = Backoff { fails: 0, last: None };
        for i in 0..5 {
            assert!(!b.failed(t + Duration::from_secs(120 * i)));
        }
    }

    #[test]
    fn upload_rows_are_aligned() {
        assert_eq!(aligned_pitch(4), 256);
        assert_eq!(aligned_pitch(256), 256);
        assert_eq!(aligned_pitch(257), 512);
        assert_eq!(aligned_pitch(1920 * 4), 7680);
    }
}

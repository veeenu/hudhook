use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use imgui;
use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId, Ui};
use memoffset::offset_of;
use tracing::{error, trace};
use windows::core::{s, w, ComInterface, Result, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, ID3DInclude, D3D_FEATURE_LEVEL_11_1, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIAdapter, IDXGIFactory2, IDXGISwapChain3, DXGI_CREATE_FACTORY_DEBUG,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::{
    CreateEventA, CreateEventExW, WaitForSingleObject, WaitForSingleObjectEx, CREATE_EVENT,
    INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, IsChild};

use super::keys;
use crate::util::{try_out_param, try_out_ptr};

const COMMAND_ALLOCATOR_NAMES: [PCWSTR; 8] = [
    w!("hudhook Command allocator #0"),
    w!("hudhook Command allocator #1"),
    w!("hudhook Command allocator #2"),
    w!("hudhook Command allocator #3"),
    w!("hudhook Command allocator #4"),
    w!("hudhook Command allocator #5"),
    w!("hudhook Command allocator #6"),
    w!("hudhook Command allocator #7"),
];

// Holds D3D12 resources for a back buffer.
#[derive(Debug)]
struct FrameContext {
    desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    back_buffer: ID3D12Resource,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_val: u64,
    fence_event: HANDLE,
}

impl FrameContext {
    unsafe fn new(
        device: &ID3D12Device,
        swap_chain: &IDXGISwapChain3,
        handle_start: D3D12_CPU_DESCRIPTOR_HANDLE,
        heap_inc_size: u32,
        index: u32,
    ) -> Result<Self> {
        let desc_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: handle_start.ptr + (index * heap_inc_size) as usize,
        };

        let back_buffer: ID3D12Resource = swap_chain.GetBuffer(index)?;
        device.CreateRenderTargetView(&back_buffer, None, desc_handle);

        let command_allocator: ID3D12CommandAllocator =
            device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
        command_allocator.SetName(COMMAND_ALLOCATOR_NAMES[index as usize % 8])?;

        Ok(FrameContext {
            desc_handle,
            back_buffer,
            command_allocator,
            fence: device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap(),
            fence_val: 0,
            fence_event: CreateEventExW(None, None, CREATE_EVENT(0), 0x1F0003).unwrap(),
        })
    }

    fn incr(&mut self) {
        static FENCE_MAX: AtomicU64 = AtomicU64::new(0);
        self.fence_val = FENCE_MAX.fetch_add(1, Ordering::SeqCst);
    }

    fn wait_fence(&mut self) {
        unsafe {
            if self.fence.GetCompletedValue() < self.fence_val {
                self.fence.SetEventOnCompletion(self.fence_val, self.fence_event).unwrap();
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
        }
    }
}

// Holds D3D12 buffers for a frame.
struct FrameResources {
    index_buffer: Option<ID3D12Resource>,
    vertex_buffer: Option<ID3D12Resource>,
    index_buffer_size: usize,
    vertex_buffer_size: usize,
    vertices: Vec<DrawVert>,
    indices: Vec<DrawIdx>,
}

impl FrameResources {
    fn resize(&mut self, dev: &ID3D12Device, indices: usize, vertices: usize) -> Result<()> {
        if self.vertex_buffer.is_none() || self.vertex_buffer_size < vertices {
            drop(self.vertex_buffer.take());

            self.vertex_buffer_size = vertices + 5000;
            let props = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 65536,
                Width: (self.vertex_buffer_size * size_of::<imgui::DrawVert>()) as u64,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            unsafe {
                dev.CreateCommittedResource(
                    &props,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    None,
                    &mut self.vertex_buffer,
                )
            }
            .map_err(|e| {
                error!("Resizing index buffer: {:?}", e);
                e
            })?;
        }

        if self.index_buffer.is_none() || self.index_buffer_size < indices {
            drop(self.index_buffer.take());
            self.index_buffer_size = indices + 10000;
            let props = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: (self.index_buffer_size * size_of::<imgui::DrawIdx>()) as u64,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            unsafe {
                dev.CreateCommittedResource(
                    &props,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    None,
                    &mut self.index_buffer,
                )
            }
            .map_err(|e| {
                error!("Resizing index buffer: {:?}", e);
                e
            })?;
        }
        Ok(())
    }
}

impl Drop for FrameResources {
    fn drop(&mut self) {
        drop(self.vertex_buffer.take());
        drop(self.index_buffer.take());
    }
}

impl Default for FrameResources {
    fn default() -> Self {
        Self {
            index_buffer: None,
            vertex_buffer: None,
            index_buffer_size: 10000,
            vertex_buffer_size: 5000,
            vertices: Default::default(),
            indices: Default::default(),
        }
    }
}

// RAII wrapper around a [`std::mem::ManuallyDrop`] for a D3D12 resource
// barrier.
struct Barrier;

impl Barrier {
    fn create(
        buf: ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) -> Vec<D3D12_RESOURCE_BARRIER> {
        let transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: ManuallyDrop::new(Some(buf)),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: before,
            StateAfter: after,
        });

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 { Transition: transition_barrier },
        };

        vec![barrier]
    }

    fn drop(barriers: Vec<D3D12_RESOURCE_BARRIER>) {
        for barrier in barriers {
            let transition = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
            let _ = ManuallyDrop::into_inner(transition.pResource);
        }
    }
}

// Holds and manages the lifetimes for the DirectComposition data structures.
struct Compositor {
    dcomp_dev: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    root_visual: IDCompositionVisual,
}

impl Compositor {
    unsafe fn new(target_hwnd: HWND) -> Result<Self> {
        let dcomp_dev: IDCompositionDevice = DCompositionCreateDevice(None)?;
        let dcomp_target = dcomp_dev.CreateTargetForHwnd(target_hwnd, BOOL::from(true))?;

        let root_visual = dcomp_dev.CreateVisual()?;
        dcomp_target.SetRoot(&root_visual)?;
        dcomp_dev.Commit()?;

        Ok(Self { dcomp_dev, _dcomp_target: dcomp_target, root_visual })
    }

    unsafe fn render(&self, swap_chain: &IDXGISwapChain3) -> Result<()> {
        self.root_visual.SetContent(swap_chain)?;
        self.dcomp_dev.Commit()?;

        Ok(())
    }
}

/// The [`hudhook`](crate) render engine.
///
/// Most of the operations of this structures are managed by the library itself
/// and are not available to the clients. For this reason, it can't be
/// instantiated directly but only by [`Hooks`](crate::Hooks) implementations
/// via [`RenderState`](crate::renderer::RenderState).
pub struct RenderEngine {
    target_hwnd: HWND,

    _dxgi_factory: IDXGIFactory2,
    _dxgi_adapter: IDXGIAdapter,

    device: ID3D12Device,
    swap_chain: IDXGISwapChain3,

    command_queue: ID3D12CommandQueue,
    command_list: ID3D12GraphicsCommandList,

    textures_heap: ID3D12DescriptorHeap,
    rtv_heap: ID3D12DescriptorHeap,

    cpu_desc: D3D12_CPU_DESCRIPTOR_HANDLE,
    gpu_desc: D3D12_GPU_DESCRIPTOR_HANDLE,
    frame_resources: Vec<FrameResources>,
    frame_contexts: Vec<FrameContext>,
    const_buf: [[f32; 4]; 4],

    ctx: Rc<RefCell<Context>>,

    font_texture_resource: Option<ID3D12Resource>,
    root_signature: Option<ID3D12RootSignature>,
    pipeline_state: Option<ID3D12PipelineState>,

    textures: Vec<(ID3D12Resource, TextureId)>,

    compositor: Compositor,
}

impl RenderEngine {
    pub(crate) fn new(target_hwnd: HWND) -> Result<Self> {
        // Build device and swap chain.
        let dxgi_factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG) }?;

        let dxgi_adapter = unsafe { dxgi_factory.EnumAdapters(0) }?;

        let mut device: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(&dxgi_adapter, D3D_FEATURE_LEVEL_11_1, &mut device) }?;
        let device = device.unwrap();

        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        };

        let command_queue: ID3D12CommandQueue =
            unsafe { device.CreateCommandQueue(&queue_desc as *const _) }.unwrap();

        let (width, height) = crate::util::win_size(target_hwnd);

        let sd = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Width: width as _,
            Height: height as _,
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };

        let swap_chain =
            unsafe { dxgi_factory.CreateSwapChainForComposition(&command_queue, &sd, None) }?
                .cast::<IDXGISwapChain3>()?;

        // Descriptor heap for textures (font + user-defined)
        let textures_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })
        }?;

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: sd.BufferCount,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                    NodeMask: 1,
                })
                .unwrap()
        };

        let rtv_heap_inc_size =
            unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) };

        let rtv_heap_handle_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        // Build frame contexts.
        let frame_contexts: Vec<FrameContext> = (0..sd.BufferCount)
            .map(|i| unsafe {
                FrameContext::new(&device, &swap_chain, rtv_heap_handle_start, rtv_heap_inc_size, i)
            })
            .collect::<Result<Vec<_>>>()?;

        let frame_resources =
            (0..sd.BufferCount).map(|_| FrameResources::default()).collect::<Vec<_>>();

        // Build command objects.
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
        }?;
        unsafe {
            command_list.Close().unwrap();
            command_list.SetName(w!("hudhook Command List"))?;
        };

        let cpu_desc = unsafe { textures_heap.GetCPUDescriptorHandleForHeapStart() };
        let gpu_desc = unsafe { textures_heap.GetGPUDescriptorHandleForHeapStart() };

        let mut ctx = Context::create();
        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("imgui-dx12@", env!("CARGO_PKG_VERSION"))));

        let ctx = Rc::new(RefCell::new(ctx));
        let compositor = unsafe { Compositor::new(target_hwnd) }?;

        Ok(Self {
            target_hwnd,
            _dxgi_factory: dxgi_factory,
            _dxgi_adapter: dxgi_adapter,
            device,
            swap_chain,
            command_queue,
            command_list,
            textures_heap,
            rtv_heap,
            cpu_desc,
            gpu_desc,
            frame_resources,
            frame_contexts,
            const_buf: [[0f32; 4]; 4],
            ctx,
            compositor,
            font_texture_resource: None,
            root_signature: None,
            pipeline_state: None,
            textures: Vec::new(),
        })
    }

    pub(crate) fn resize(&mut self) -> Result<()> {
        let (width, height) = crate::util::win_size(self.target_hwnd);

        self.frame_contexts.drain(..).for_each(drop);
        self.frame_resources.drain(..).for_each(drop);

        let mut sd = Default::default();
        unsafe { self.swap_chain.GetDesc(&mut sd)? };
        unsafe {
            self.swap_chain.ResizeBuffers(
                sd.BufferCount,
                width as _,
                height as _,
                sd.BufferDesc.Format,
                sd.Flags,
            )?
        };

        let rtv_heap_inc_size =
            unsafe { self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) };

        let rtv_heap_handle_start = unsafe { self.rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        // Build frame contexts.
        self.frame_contexts = (0..sd.BufferCount)
            .map(|i| unsafe {
                FrameContext::new(
                    &self.device,
                    &self.swap_chain,
                    rtv_heap_handle_start,
                    rtv_heap_inc_size,
                    i,
                )
            })
            .collect::<Result<Vec<_>>>()?;

        self.frame_resources = (0..sd.BufferCount).map(|_| FrameResources::default()).collect();

        Ok(())
    }

    pub(crate) fn render<F: FnMut(&mut Ui)>(&mut self, mut render_loop: F) -> Result<()> {
        unsafe { self.setup_io()? };

        // Create device objects if necessary.
        if self.pipeline_state.is_none() {
            unsafe { self.create_device_objects() }?;
        }

        let idx = unsafe { self.swap_chain.GetCurrentBackBufferIndex() } as usize;
        self.frame_contexts[idx].wait_fence();
        self.frame_contexts[idx].incr();

        let command_allocator = &self.frame_contexts[idx].command_allocator;

        // Reset command allocator and list state.
        unsafe {
            command_allocator.Reset().unwrap();
            self.command_list.Reset(command_allocator, None).unwrap();
        }

        // Setup a barrier that waits for the back buffer to transition to a render
        // target.
        let back_buffer_to_rt_barrier = Barrier::create(
            self.frame_contexts[idx].back_buffer.clone(),
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );

        unsafe {
            self.command_list.ResourceBarrier(&back_buffer_to_rt_barrier);

            // Setup the back buffer as a render target and clear it.
            self.command_list.OMSetRenderTargets(
                1,
                Some(&self.frame_contexts[idx].desc_handle),
                BOOL::from(false),
                None,
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.textures_heap.clone())]);
            self.command_list.ClearRenderTargetView(
                self.frame_contexts[idx].desc_handle,
                &[0.0, 0.0, 0.0, 0.0],
                None,
            );
        }

        // Draw data loop.
        let ctx = Rc::clone(&self.ctx);
        let mut ctx = ctx.borrow_mut();
        let ui = ctx.frame();
        render_loop(ui);
        let draw_data = ctx.render();

        if let Err(e) = unsafe { self.render_draw_data(draw_data, idx) } {
            eprintln!("{}", e);
        };

        // Setup a barrier to wait for the back buffer to transition to presentable
        // state.
        let back_buffer_to_present_barrier = Barrier::create(
            self.frame_contexts[idx].back_buffer.clone(),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );

        unsafe {
            self.command_list.ResourceBarrier(&back_buffer_to_present_barrier);

            // Close and execute the command list.
            self.command_list.Close()?;
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue
                .Signal(&self.frame_contexts[idx].fence, self.frame_contexts[idx].fence_val)?;
        }

        unsafe {
            // Present the content.
            self.swap_chain.Present(1, 0).ok()?;
        };

        // Drop the barriers.
        Barrier::drop(back_buffer_to_rt_barrier);
        Barrier::drop(back_buffer_to_present_barrier);

        // Composite the frame over the hwnd.
        unsafe { self.compositor.render(&self.swap_chain) }?;

        Ok(())
    }
}

impl RenderEngine {
    /// Returns the [`HWND`] the UI is composited on top of.
    pub fn hwnd(&self) -> HWND {
        self.target_hwnd
    }

    /// Returns an internally mutable reference to the active
    /// [`imgui::Context`].
    pub fn ctx(&mut self) -> Rc<RefCell<Context>> {
        Rc::clone(&self.ctx)
    }

    /// Upload an image into a texture and return a [`imgui::TextureId`].
    ///
    /// Should be used ideally ahead-of-time in
    /// [`ImguiRenderLoop::initialize`](crate::ImguiRenderLoop::initialize), but
    /// can be used in
    /// [`ImguiRenderLoop::before_render`](crate::ImguiRenderLoop::before_render)
    /// provided the performance cost is acceptable.
    pub fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe { self.resize_texture_heap()? };

        let (p_texture, tex_id) =
            self.create_texture_inner(data, width, height, self.textures.len() as u32 + 1)?;

        self.textures.push((p_texture, tex_id));

        Ok(tex_id)
    }
}

impl RenderEngine {
    unsafe fn setup_io(&mut self) -> Result<()> {
        let sd = try_out_param(|sd| unsafe { self.swap_chain.GetDesc1(sd) })?;

        let mut ctx = self.ctx.borrow_mut();

        // Setup display size and cursor position.
        let io = ctx.io_mut();

        io.display_size = [sd.Width as f32, sd.Height as f32];

        let active_window = unsafe { GetForegroundWindow() };
        if !HANDLE(active_window.0).is_invalid()
            && (active_window == self.target_hwnd
                || unsafe { IsChild(active_window, self.target_hwnd) }.as_bool())
        {
            let mut pos = Default::default();
            let gcp = unsafe { GetCursorPos(&mut pos) };
            if gcp.is_ok()
                && unsafe { ScreenToClient(self.target_hwnd, &mut pos as *mut _) }.as_bool()
            {
                io.mouse_pos[0] = pos.x as _;
                io.mouse_pos[1] = pos.y as _;
            }
        }

        io.nav_active = true;
        io.nav_visible = true;

        // Map key indices to the virtual key codes
        for i in keys::KEYS {
            io[i.0] = i.1 .0 as _;
        }

        Ok(())
    }

    unsafe fn create_device_objects(&mut self) -> Result<()> {
        if self.pipeline_state.is_some() {
            self.invalidate_device_objects();
        }

        let desc_range = D3D12_DESCRIPTOR_RANGE {
            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
            NumDescriptors: 1,
            BaseShaderRegister: 0,
            RegisterSpace: 0,
            OffsetInDescriptorsFromTableStart: 0,
        };

        let params = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Constants: D3D12_ROOT_CONSTANTS {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                        Num32BitValues: 16,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &desc_range,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ];

        let sampler = D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            MipLODBias: 0f32,
            MaxAnisotropy: 0,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
            MinLOD: 0f32,
            MaxLOD: 0f32,
            ShaderRegister: 0,
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        };

        let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: 2,
            pParameters: params.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS,
        };
        let mut blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;
        if let Err(e) = D3D12SerializeRootSignature(
            &root_signature_desc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            Some(&mut err_blob),
        ) {
            if let Some(err_blob) = err_blob {
                let buf_ptr = unsafe { err_blob.GetBufferPointer() } as *mut u8;
                let buf_size = unsafe { err_blob.GetBufferSize() };
                let s = unsafe { String::from_raw_parts(buf_ptr, buf_size, buf_size + 1) };
                error!("Serializing root signature: {}: {}", e, s);
            }
            return Err(e);
        }

        let blob = blob.unwrap();
        self.root_signature = Some(self.device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()),
        )?);

        let vs = r#"
                cbuffer vertexBuffer : register(b0)
                {
                  float4x4 ProjectionMatrix;
                };
                struct VS_INPUT
                {
                  float2 pos : POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };

                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };

                PS_INPUT main(VS_INPUT input)
                {
                  PS_INPUT output;
                  output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
                  output.col = input.col;
                  output.uv  = input.uv;
                  return output;
                }"#;

        let vtx_shader: ID3DBlob = try_out_ptr(|v| unsafe {
            D3DCompile(
                vs.as_ptr() as _,
                vs.len(),
                None,
                None,
                None::<&ID3DInclude>,
                s!("main\0"),
                s!("vs_5_0\0"),
                0,
                0,
                v,
                None,
            )
        })
        .expect("D3DCompile vertex shader");

        let ps = r#"
                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };
                SamplerState sampler0 : register(s0);
                Texture2D texture0 : register(t0);

                float4 main(PS_INPUT input) : SV_Target
                {
                  float4 out_col = input.col * texture0.Sample(sampler0, input.uv);
                  return out_col;
                }"#;

        let pix_shader = try_out_ptr(|v| unsafe {
            D3DCompile(
                ps.as_ptr() as _,
                ps.len(),
                None,
                None,
                None::<&ID3DInclude>,
                s!("main\0"),
                s!("ps_5_0\0"),
                0,
                0,
                v,
                None,
            )
        })
        .expect("D3DCompile pixel shader");

        let root_signature = ManuallyDrop::new(self.root_signature.clone());
        let mut pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: root_signature,
            NodeMask: 1,
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            SampleMask: u32::MAX,
            NumRenderTargets: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
            ..Default::default()
        };
        pso_desc.RTVFormats[0] = DXGI_FORMAT_B8G8R8A8_UNORM;
        pso_desc.DSVFormat = DXGI_FORMAT_D32_FLOAT;

        pso_desc.VS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { vtx_shader.GetBufferPointer() },
            BytecodeLength: unsafe { vtx_shader.GetBufferSize() },
        };

        pso_desc.PS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { pix_shader.GetBufferPointer() },
            BytecodeLength: unsafe { pix_shader.GetBufferSize() },
        };

        let elem_descs = [
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("POSITION\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, pos) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("TEXCOORD\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, uv) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("COLOR\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, col) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];

        pso_desc.InputLayout =
            D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elem_descs.as_ptr(), NumElements: 3 };

        pso_desc.BlendState.AlphaToCoverageEnable = BOOL::from(false);
        pso_desc.BlendState.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            LogicOpEnable: false.into(),
            SrcBlend: D3D12_BLEND_SRC_ALPHA,
            DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_ONE,
            DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: Default::default(),
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as _,
        };
        pso_desc.RasterizerState = D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: false.into(),
            DepthBias: D3D12_DEFAULT_DEPTH_BIAS,
            DepthBiasClamp: D3D12_DEFAULT_DEPTH_BIAS_CLAMP,
            SlopeScaledDepthBias: D3D12_DEFAULT_SLOPE_SCALED_DEPTH_BIAS,
            DepthClipEnable: true.into(),
            MultisampleEnable: false.into(),
            AntialiasedLineEnable: false.into(),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        };

        let pipeline_state = unsafe { self.device.CreateGraphicsPipelineState(&pso_desc) };
        self.pipeline_state = Some(pipeline_state.unwrap());

        self.create_font_texture()?;

        Ok(())
    }

    unsafe fn resize_texture_heap(&mut self) -> Result<()> {
        let mut desc = self.textures_heap.GetDesc();
        let old_num_descriptors = desc.NumDescriptors;
        if (old_num_descriptors as usize) < self.textures.len() {
            desc.NumDescriptors *= 2;
            let new_texture_heap: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&desc)?;
            self.device.CopyDescriptorsSimple(
                old_num_descriptors,
                new_texture_heap.GetCPUDescriptorHandleForHeapStart(),
                self.textures_heap.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            self.textures_heap = new_texture_heap;
        }

        Ok(())
    }

    fn create_texture_inner(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        tex_index: u32,
    ) -> Result<(ID3D12Resource, TextureId)> {
        let heap_inc_size = unsafe {
            self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
        };

        let cpu_desc = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.cpu_desc.ptr + (tex_index * heap_inc_size) as usize,
        };

        let gpu_desc = D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: self.gpu_desc.ptr + (tex_index * heap_inc_size) as u64,
        };

        let p_texture: ManuallyDrop<Option<ID3D12Resource>> =
            ManuallyDrop::new(try_out_param(|v| unsafe {
                self.device.CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES {
                        Type: D3D12_HEAP_TYPE_DEFAULT,
                        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                        CreationNodeMask: Default::default(),
                        VisibleNodeMask: Default::default(),
                    },
                    D3D12_HEAP_FLAG_NONE,
                    &D3D12_RESOURCE_DESC {
                        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                        Alignment: 0,
                        Width: width as _,
                        Height: height as _,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                        Flags: D3D12_RESOURCE_FLAG_NONE,
                    },
                    D3D12_RESOURCE_STATE_COPY_DEST,
                    None,
                    v,
                )
            })?);

        let mut upload_buffer: Option<ID3D12Resource> = None;
        let upload_pitch = width * 4;
        let upload_size = height * upload_pitch;
        unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: Default::default(),
                    VisibleNodeMask: Default::default(),
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: upload_size as _,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                &mut upload_buffer,
            )
        }?;
        let upload_buffer = ManuallyDrop::new(upload_buffer);

        let range = D3D12_RANGE { Begin: 0, End: upload_size as usize };
        if let Some(ub) = upload_buffer.as_ref() {
            unsafe {
                let mut ptr: *mut u8 = null_mut();
                ub.Map(0, Some(&range), Some(&mut ptr as *mut _ as *mut *mut c_void)).unwrap();
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                ub.Unmap(0, Some(&range));
            }
        };

        let fence: ID3D12Fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;

        let event =
            unsafe { CreateEventA(None, BOOL::from(false), BOOL::from(false), PCSTR(null())) }?;

        let cmd_allocator: ID3D12CommandAllocator =
            unsafe { self.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        unsafe { cmd_allocator.SetName(w!("hudhook font texture Command Allocator")) }?;

        let cmd_list: ID3D12GraphicsCommandList = unsafe {
            self.device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &cmd_allocator, None)
        }?;

        unsafe { cmd_list.SetName(w!("hudhook font texture Command List")) }?;

        let src_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: upload_buffer,
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        Width: width,
                        Height: height,
                        Depth: 1,
                        RowPitch: upload_pitch,
                    },
                },
            },
        };

        let dst_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: p_texture.clone(),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
        };

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: p_texture.clone(),
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                }),
            },
        };

        unsafe {
            cmd_list.CopyTextureRegion(&dst_location, 0, 0, 0, &src_location, None);
            cmd_list.ResourceBarrier(&[barrier]);
            cmd_list.Close().unwrap();
            self.command_queue.ExecuteCommandLists(&[Some(cmd_list.cast()?)]);
            self.command_queue.Signal(&fence, 1)?;
            fence.SetEventOnCompletion(1, event)?;
            WaitForSingleObject(event, u32::MAX);
        };

        let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                    PlaneSlice: Default::default(),
                    ResourceMinLODClamp: Default::default(),
                },
            },
        };

        unsafe { CloseHandle(event)? };

        unsafe {
            self.device.CreateShaderResourceView(p_texture.as_ref(), Some(&srv_desc), cpu_desc)
        };

        let tex_id = TextureId::from(gpu_desc.ptr as usize);

        drop(ManuallyDrop::into_inner(src_location.pResource));

        Ok((ManuallyDrop::into_inner(p_texture).unwrap(), tex_id))
    }

    unsafe fn create_font_texture(&mut self) -> Result<()> {
        let ctx = Rc::clone(&self.ctx);
        let mut ctx = ctx.borrow_mut();
        let fonts = ctx.fonts();
        let texture = fonts.build_rgba32_texture();

        self.resize_texture_heap()?;
        let (p_texture, tex_id) =
            self.create_texture_inner(texture.data, texture.width, texture.height, 0)?;

        drop(self.font_texture_resource.take());
        self.font_texture_resource = Some(p_texture);
        fonts.tex_id = tex_id;

        Ok(())
    }

    fn invalidate_device_objects(&mut self) {
        if let Some(root_signature) = self.root_signature.take() {
            drop(root_signature);
        }
        if let Some(pipeline_state) = self.pipeline_state.take() {
            drop(pipeline_state);
        }
        if let Some(font_texture_resource) = self.font_texture_resource.take() {
            drop(font_texture_resource);
        }

        self.frame_resources.iter_mut().for_each(|fr| {
            drop(fr.index_buffer.take());
            drop(fr.vertex_buffer.take());
        });
    }

    unsafe fn setup_render_state(&mut self, draw_data: &DrawData, idx: usize) {
        let display_pos = draw_data.display_pos;
        let display_size = draw_data.display_size;

        let frame_resources = &self.frame_resources[idx];
        self.const_buf = {
            let [l, t, r, b] = [
                display_pos[0],
                display_pos[1],
                display_pos[0] + display_size[0],
                display_pos[1] + display_size[1],
            ];

            [[2. / (r - l), 0., 0., 0.], [0., 2. / (t - b), 0., 0.], [0., 0., 0.5, 0.], [
                (r + l) / (l - r),
                (t + b) / (b - t),
                0.5,
                1.0,
            ]]
        };

        trace!("Display size {}x{}", display_size[0], display_size[1]);
        self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
            TopLeftX: 0f32,
            TopLeftY: 0f32,
            Width: display_size[0],
            Height: display_size[1],
            MinDepth: 0f32,
            MaxDepth: 1f32,
        }]);

        self.command_list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: frame_resources
                    .vertex_buffer
                    .as_ref()
                    .unwrap()
                    .GetGPUVirtualAddress(),
                SizeInBytes: (frame_resources.vertex_buffer_size * size_of::<DrawVert>()) as _,
                StrideInBytes: size_of::<DrawVert>() as _,
            }]),
        );

        self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: frame_resources.index_buffer.as_ref().unwrap().GetGPUVirtualAddress(),
            SizeInBytes: (frame_resources.index_buffer_size * size_of::<DrawIdx>()) as _,
            Format: if size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
        }));
        self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.command_list.SetPipelineState(self.pipeline_state.as_ref().unwrap());
        self.command_list.SetGraphicsRootSignature(self.root_signature.as_ref().unwrap());
        self.command_list.SetGraphicsRoot32BitConstants(
            0,
            16,
            self.const_buf.as_ptr() as *const c_void,
            0,
        );
        self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));
    }

    unsafe fn render_draw_data(&mut self, draw_data: &DrawData, idx: usize) -> Result<()> {
        let print_device_removed_reason = |e: windows::core::Error| -> windows::core::Error {
            trace!("Device removed reason: {:?}", unsafe { self.device.GetDeviceRemovedReason() });
            e
        };

        if draw_data.display_size[0] <= 0f32 || draw_data.display_size[1] <= 0f32 {
            trace!(
                "Insufficent display size {}x{}, skip rendering",
                draw_data.display_size[0],
                draw_data.display_size[1]
            );
            return Ok(());
        }

        self.frame_resources[idx]
            .resize(
                &self.device,
                draw_data.total_idx_count as usize,
                draw_data.total_vtx_count as usize,
            )
            .map_err(print_device_removed_reason)?;

        let range = D3D12_RANGE::default();
        let mut vtx_resource: *mut imgui::DrawVert = null_mut();
        let mut idx_resource: *mut imgui::DrawIdx = null_mut();

        self.frame_resources[idx].vertices.clear();
        self.frame_resources[idx].indices.clear();
        draw_data.draw_lists().map(|m| (m.vtx_buffer().iter(), m.idx_buffer().iter())).for_each(
            |(v, i)| {
                self.frame_resources[idx].vertices.extend(v);
                self.frame_resources[idx].indices.extend(i);
            },
        );

        let vertices = &self.frame_resources[idx].vertices;
        let indices = &self.frame_resources[idx].indices;

        {
            let frame_resources = &self.frame_resources[idx];

            if let Some(vb) = frame_resources.vertex_buffer.as_ref() {
                vb.Map(0, Some(&range), Some(&mut vtx_resource as *mut _ as *mut *mut c_void))
                    .map_err(print_device_removed_reason)?;
                std::ptr::copy_nonoverlapping(vertices.as_ptr(), vtx_resource, vertices.len());
                vb.Unmap(0, Some(&range));
            };

            if let Some(ib) = frame_resources.index_buffer.as_ref() {
                ib.Map(0, Some(&range), Some(&mut idx_resource as *mut _ as *mut *mut c_void))
                    .map_err(print_device_removed_reason)?;
                std::ptr::copy_nonoverlapping(indices.as_ptr(), idx_resource, indices.len());
                ib.Unmap(0, Some(&range));
            };
        }

        self.setup_render_state(draw_data, idx);

        let mut vtx_offset = 0usize;
        let mut idx_offset = 0usize;

        for cl in draw_data.draw_lists() {
            for cmd in cl.commands() {
                match cmd {
                    DrawCmd::Elements { count, cmd_params } => {
                        let [cx, cy, cw, ch] = cmd_params.clip_rect;
                        let [x, y] = draw_data.display_pos;
                        let r = RECT {
                            left: (cx - x) as i32,
                            top: (cy - y) as i32,
                            right: (cw - x) as i32,
                            bottom: (ch - y) as i32,
                        };

                        if r.right > r.left && r.bottom > r.top {
                            let tex_handle = D3D12_GPU_DESCRIPTOR_HANDLE {
                                ptr: cmd_params.texture_id.id() as _,
                            };
                            unsafe {
                                self.command_list.SetGraphicsRootDescriptorTable(1, tex_handle);
                                self.command_list.RSSetScissorRects(&[r]);
                                self.command_list.DrawIndexedInstanced(
                                    count as _,
                                    1,
                                    (cmd_params.idx_offset + idx_offset) as _,
                                    (cmd_params.vtx_offset + vtx_offset) as _,
                                    0,
                                );
                            }
                        }
                    },
                    DrawCmd::ResetRenderState => {
                        self.setup_render_state(draw_data, idx);
                    },
                    DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
                        callback(cl.raw(), raw_cmd)
                    },
                }
            }
            idx_offset += cl.idx_buffer().len();
            vtx_offset += cl.vtx_buffer().len();
        }
        Ok(())
    }
}

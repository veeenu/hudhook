use std::cell::RefCell;
use std::ffi::c_void;
use std::mem;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, info, trace};
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandList, ID3D12CommandQueue, ID3D12Device,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, IDXGISwapChain, IDXGISwapChain3, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use super::DummyHwnd;
use crate::compositor::dx12::Compositor;
use crate::compositor::dx12_new;
use crate::mh::MhHook;
use crate::renderer::{engine_new, print_dxgi_debug_messages, RenderState};
use crate::util::{self, try_out_ptr};
use crate::{Hooks, ImguiRenderLoop};

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain3, SyncInterval: u32, Flags: u32) -> HRESULT;

type DXGISwapChainResizeBuffersType = unsafe extern "system" fn(
    This: IDXGISwapChain3,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT;

type D3D12CommandQueueExecuteCommandListsType = unsafe extern "system" fn(
    This: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
);

struct Trampolines {
    dxgi_swap_chain_present: DXGISwapChainPresentType,
    dxgi_swap_chain_resize_buffers: DXGISwapChainResizeBuffersType,
    d3d12_command_queue_execute_command_lists: D3D12CommandQueueExecuteCommandListsType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut COMPOSITOR: OnceLock<Mutex<Compositor>> = OnceLock::new();

static mut PIPELINE: OnceCell<Mutex<Pipeline>> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Mutex<Box<dyn ImguiRenderLoop + Send + Sync>>> = OnceCell::new();

struct Pipeline {
    ctx: Rc<RefCell<Context>>,
    compositor: dx12_new::Compositor,
    engine: engine_new::RenderEngine,
}

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    swap_chain: IDXGISwapChain3,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_present, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    trace!("PIPELINE");
    let Some(mut pipeline) = PIPELINE.get().and_then(Mutex::try_lock) else {
        trace!("Could not lock pipeline in Present");
        print_dxgi_debug_messages();
        return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
    };

    trace!("RENDER LOOP");
    let Some(mut render_loop) = RENDER_LOOP.get().and_then(Mutex::try_lock) else {
        trace!("Could not lock render loop in Present");
        print_dxgi_debug_messages();
        return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
    };

    // TODO store hwnd
    let desc = util::try_out_param(|v| swap_chain.GetDesc(v)).unwrap();

    // TODO wrong type
    // render_loop.before_render(&mut pipeline.engine);

    trace!("RENDER SETUP");
    let ctx = Rc::clone(&pipeline.ctx);
    if let Err(e) = pipeline.engine.render_setup(desc.OutputWindow, &mut ctx.borrow_mut()) {
        error!("Render setup: {e:?}");
        print_dxgi_debug_messages();
        return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
    }

    trace!("RENDER CTX");
    let mut ctx = ctx.borrow_mut();

    if ctx.io().display_size[0] <= 0.0 || ctx.io().display_size[1] <= 0.0 {
        error!("{:?}", ctx.io().display_size);
        print_dxgi_debug_messages();
        return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
    }

    let ui = ctx.frame();
    render_loop.render(ui);
    let draw_data = ctx.render();

    trace!("RENDER");
    let resource = match pipeline.engine.render(desc.OutputWindow, draw_data) {
        Ok(resource) => resource,
        Err(e) => {
            print_dxgi_debug_messages();
            error!("Render: {e:?}");
            return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
        },
    };

    trace!("COMPOSITE");
    if let Err(e) = pipeline
        .compositor
        .composite(resource, swap_chain.GetBuffer(swap_chain.GetCurrentBackBufferIndex()).unwrap())
    {
        error!("Composite: {e:?}");
        print_dxgi_debug_messages();
        return dxgi_swap_chain_present(swap_chain, sync_interval, flags);
    }

    // Don't attempt a render if one is already underway: it might be that the
    // renderer itself is currently invoking `Present`.
    // if RenderState::is_locked() {
    //     return dxgi_swap_chain_present(p_this, sync_interval, flags);
    // }
    //
    // let hwnd = RenderState::setup(|| {
    //     let mut desc = Default::default();
    //     p_this.GetDesc(&mut desc).unwrap();
    //     info!("Output window: {:?}", p_this);
    //     info!("Desc: {:?}", desc);
    //     desc.OutputWindow
    // });
    //
    // RenderState::lock();
    // let surface = RenderState::render(hwnd);
    // let mut compositor = COMPOSITOR.get_or_init(||
    // Mutex::new(Compositor::new())).lock();
    //
    // if let Err(e) = compositor.with_swap_chain(&p_this) {
    //     error!("Could not initialize compositor's swap chain: {e:?}");
    // }
    //
    // if let Some(surface) = surface {
    //     if let Err(e) = compositor.composite(surface.resource) {
    //         error!("Could not composite: {e:?}");
    //     }
    // }
    //
    // drop(compositor);
    // RenderState::unlock();

    trace!("Call IDXGISwapChain::Present trampoline");
    dxgi_swap_chain_present(swap_chain, sync_interval, flags)
}

unsafe extern "system" fn dxgi_swap_chain_resize_buffers_impl(
    p_this: IDXGISwapChain3,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_resize_buffers, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    trace!("Call IDXGISwapChain::ResizeBuffers trampoline");
    dxgi_swap_chain_resize_buffers(p_this, buffer_count, width, height, new_format, flags)
}

unsafe extern "system" fn d3d12_command_queue_execute_command_lists_impl(
    command_queue: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
) {
    trace!(
        "ID3D12CommandQueue::ExecuteCommandLists({command_queue:?}, {num_command_lists}, \
         {command_lists:p}) invoked"
    );

    let Trampolines { d3d12_command_queue_execute_command_lists, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    static LOCK: AtomicBool = AtomicBool::new(false);

    if LOCK.load(Ordering::SeqCst) {
        return d3d12_command_queue_execute_command_lists(
            command_queue,
            num_command_lists,
            command_lists,
        );
    }

    if let Err(e) = PIPELINE.get_or_try_init(|| -> windows::core::Result<_> {
        LOCK.store(true, Ordering::SeqCst);
        trace!("Context create");
        let mut ctx = Context::create();
        ctx.io_mut().display_size = [800., 600.];
        trace!("Render engine new");
        let engine = engine_new::RenderEngine::new(&mut ctx, 800, 600)?;
        trace!("Compositor new");
        let compositor = dx12_new::Compositor::new(command_queue.clone())?;

        Ok(Mutex::new(Pipeline { ctx: Rc::new(RefCell::new(ctx)), compositor, engine }))
    }) {
        LOCK.store(false, Ordering::SeqCst);
        error!("Could not initialize rendering pipeline: {e:?}");
    };

    LOCK.store(false, Ordering::SeqCst);

    // if RenderState::is_locked() {
    //     return d3d12_command_queue_execute_command_lists(
    //         cmd_queue,
    //         num_command_lists,
    //         command_lists,
    //     );
    // }
    //
    // let mut compositor = COMPOSITOR.get_or_init(||
    // Mutex::new(Compositor::new())).lock();
    //
    // if let Err(e) = compositor.with_command_queue(&cmd_queue) {
    //     error!("Could not initialize compositor command queue: {e:?}");
    // }
    //
    // drop(compositor);

    d3d12_command_queue_execute_command_lists(command_queue, num_command_lists, command_lists);
}

fn get_target_addrs() -> (
    DXGISwapChainPresentType,
    DXGISwapChainResizeBuffersType,
    D3D12CommandQueueExecuteCommandListsType,
) {
    let dummy_hwnd = DummyHwnd::new();

    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter = unsafe { factory.EnumAdapters(0) }.unwrap();

    let dev: ID3D12Device =
        try_out_ptr(|v| unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, v) })
            .expect("D3D12CreateDevice failed");

    let command_queue: ID3D12CommandQueue = unsafe {
        dev.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        })
    }
    .unwrap();

    let swap_chain: IDXGISwapChain = match try_out_ptr(|v| unsafe {
        factory
            .CreateSwapChain(
                &command_queue,
                &DXGI_SWAP_CHAIN_DESC {
                    BufferDesc: DXGI_MODE_DESC {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                        Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                        Width: 640,
                        Height: 480,
                        RefreshRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    OutputWindow: dummy_hwnd.hwnd(),
                    Windowed: BOOL(1),
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as _,
                },
                v,
            )
            .ok()
    }) {
        Ok(swap_chain) => swap_chain,
        Err(e) => {
            unsafe { print_dxgi_debug_messages() };
            panic!("{e:?}");
        },
    };

    let present_ptr: DXGISwapChainPresentType =
        unsafe { mem::transmute(swap_chain.vtable().Present) };
    let resize_buffers_ptr: DXGISwapChainResizeBuffersType =
        unsafe { mem::transmute(swap_chain.vtable().ResizeBuffers) };
    let cqecl_ptr: D3D12CommandQueueExecuteCommandListsType =
        unsafe { mem::transmute(command_queue.vtable().ExecuteCommandLists) };

    (present_ptr, resize_buffers_ptr, cqecl_ptr)
}

pub struct ImguiDx12Hooks([MhHook; 3]);

impl ImguiDx12Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain3::Present`
    /// - `IDXGISwapChain3::ResizeBuffers`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let (
            dxgi_swap_chain_present_addr,
            dxgi_swap_chain_resize_buffers_addr,
            d3d12_command_queue_execute_command_lists_addr,
        ) = get_target_addrs();

        trace!("IDXGISwapChain::Present = {:p}", dxgi_swap_chain_present_addr as *const c_void);
        let hook_present = MhHook::new(
            dxgi_swap_chain_present_addr as *mut _,
            dxgi_swap_chain_present_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::Present hook");
        let hook_resize_buffers = MhHook::new(
            dxgi_swap_chain_resize_buffers_addr as *mut _,
            dxgi_swap_chain_resize_buffers_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::ResizeBuffers hook");
        let hook_cqecl = MhHook::new(
            d3d12_command_queue_execute_command_lists_addr as *mut _,
            d3d12_command_queue_execute_command_lists_impl as *mut _,
        )
        .expect("couldn't create ID3D12CommandQueue::ExecuteCommandLists hook");

        // TODO
        // RenderState::set_render_loop(t);

        RENDER_LOOP.get_or_init(|| Mutex::new(Box::new(t)));

        TRAMPOLINES.get_or_init(|| Trampolines {
            dxgi_swap_chain_present: mem::transmute(hook_present.trampoline()),
            dxgi_swap_chain_resize_buffers: mem::transmute(hook_resize_buffers.trampoline()),
            d3d12_command_queue_execute_command_lists: mem::transmute(hook_cqecl.trampoline()),
        });

        Self([hook_present, hook_resize_buffers, hook_cqecl])
    }
}

impl Hooks for ImguiDx12Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        Box::new(unsafe { Self::new(t) })
    }

    fn hooks(&self) -> &[MhHook] {
        &self.0
    }

    unsafe fn unhook(&mut self) {
        RenderState::cleanup();
        TRAMPOLINES.take();
    }
}

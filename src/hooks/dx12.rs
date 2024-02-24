use std::ffi::c_void;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, HRESULT};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandList, ID3D12CommandQueue, ID3D12Device, ID3D12Resource,
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
use windows::Win32::UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW};

use super::DummyHwnd;
use crate::compositor::dx12_compute::Compositor;
use crate::mh::MhHook;
use crate::pipeline::{Pipeline, PipelineMessage, PipelineSharedState};
use crate::renderer::print_dxgi_debug_messages;
use crate::{util, Hooks, ImguiRenderLoop};

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

static mut PIPELINE: OnceCell<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> =
    OnceCell::new();
static mut COMMAND_QUEUE: OnceCell<ID3D12CommandQueue> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(
    swap_chain: &IDXGISwapChain3,
) -> Result<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> {
    let Some(command_queue) = COMMAND_QUEUE.get() else {
        return Err(Error::new(HRESULT(-1), "Command queue not yet initialized".into()));
    };

    let device: ID3D12Device = util::try_out_ptr(|v| unsafe { command_queue.GetDevice(v) })?;
    let compositor = Compositor::new(&device)?;

    let hwnd = util::try_out_param(|v| swap_chain.GetDesc(v)).map(|desc| desc.OutputWindow)?;

    let Some(render_loop) = RENDER_LOOP.take() else {
        return Err(Error::new(HRESULT(-1), "Render loop not yet initialized".into()));
    };

    let (pipeline, shared_state) = Pipeline::new(hwnd, imgui_wnd_proc, compositor, render_loop)
        .map_err(|(e, render_loop)| {
            RENDER_LOOP.get_or_init(move || render_loop);
            e
        })?;

    Ok((Mutex::new(pipeline), shared_state))
}

fn render(swap_chain: &IDXGISwapChain3) -> Result<()> {
    let (pipeline, _) = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(swap_chain)) }?;

    let Some(mut pipeline) = pipeline.try_lock() else {
        return Err(Error::new(HRESULT(-1), "Could not lock pipeline".into()));
    };

    let source = pipeline.render()?;
    let target: ID3D12Resource =
        unsafe { swap_chain.GetBuffer(swap_chain.GetCurrentBackBufferIndex())? };

    pipeline.compositor_mut().composite(source, target)?;

    Ok(())
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some((_, shared_state)) = PIPELINE.get() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    let _ = shared_state.tx.send(PipelineMessage(hwnd, msg, wparam, lparam));

    // CONCURRENCY: as the message interpretation now happens out of band, this
    // expresses the intent as of *before* the current message was received.
    let should_block_messages = shared_state.should_block_events.load(Ordering::SeqCst);

    if should_block_messages {
        LRESULT(1)
    } else {
        CallWindowProcW(Some(shared_state.wnd_proc), hwnd, msg, wparam, lparam)
    }
}

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    swap_chain: IDXGISwapChain3,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_present, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    if let Err(e) = render(&swap_chain) {
        print_dxgi_debug_messages();
        error!("Render error: {e:?}");
    }

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
         {command_lists:p}) invoked",
    );

    let Trampolines { d3d12_command_queue_execute_command_lists, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    // TODO check command queue type
    COMMAND_QUEUE.get_or_init(|| command_queue.clone());

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
        util::try_out_ptr(|v| unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, v) })
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

    let swap_chain: IDXGISwapChain = match util::try_out_ptr(|v| unsafe {
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
    /// - `ID3D12CommandQueue::ExecuteCommandLists`
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

        RENDER_LOOP.get_or_init(|| Box::new(t));

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
        TRAMPOLINES.take();
        PIPELINE.take();
        COMMAND_QUEUE.take();
        RENDER_LOOP.take(); // should already be null
    }
}

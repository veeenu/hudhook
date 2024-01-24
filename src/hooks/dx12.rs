use std::{
    ffi::c_void,
    mem,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use parking_lot::Mutex;
use tracing::{debug, error, info, trace};
use windows::{
    core::{Interface, HRESULT},
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            Direct3D::D3D_FEATURE_LEVEL_11_0,
            Direct3D12::{
                D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device,
                D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC,
                D3D12_COMMAND_QUEUE_FLAG_NONE,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC,
                    DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, DXGIGetDebugInterface1, IDXGIFactory1, IDXGIInfoQueue,
                IDXGISwapChain, IDXGISwapChain3, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
                DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH,
                DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
        UI::WindowsAndMessaging::{DefWindowProcW, GWLP_WNDPROC},
    },
};

use crate::renderer::dx12::RenderEngine;
use crate::Hooks;
use crate::ImguiRenderLoop;
use crate::{mh::MhHook, util::try_out_ptr};

use super::{
    input::{imgui_wnd_proc_impl, WndProcType},
    DummyHwnd,
};

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

// type DXGICommandQueueExecuteCommandListsType = unsafe extern "system" fn(
//     This: ID3D12CommandQueue,
//     num_command_lists: u32,
//     command_lists: *mut ID3D12CommandList,
// );

struct Trampolines {
    dxgi_swap_chain_present: DXGISwapChainPresentType,
    dxgi_swap_chain_resize_buffers: DXGISwapChainResizeBuffersType,
    // dxgi_command_queue_execute_command_lists: DXGICommandQueueExecuteCommandListsType,
}

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut WND_PROC: OnceLock<WndProcType> = OnceLock::new();
static mut RENDER_ENGINE: OnceLock<Mutex<RenderEngine>> = OnceLock::new();
static mut RENDER_LOOP: OnceLock<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceLock::new();
static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static RENDER_LOCK: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain3,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_present, .. } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    // Don't attempt a render if one is already underway: it might be that the renderer itself
    // is currently invoking `Present`.
    if RENDER_LOCK.load(Ordering::SeqCst) {
        return dxgi_swap_chain_present(p_this, sync_interval, flags);
    }

    let hwnd = *GAME_HWND.get_or_init(|| {
        let mut desc = Default::default();
        p_this.GetDesc(&mut desc).unwrap();
        info!("Output window: {:?}", p_this);
        info!("Desc: {:?}", desc);
        desc.OutputWindow
    });

    WND_PROC.get_or_init(|| {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let wnd_proc = unsafe {
            mem::transmute(windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA(
                hwnd,
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as isize,
            ))
        };

        #[cfg(target_arch = "x86")]
        let wnd_proc = unsafe {
            mem::transmute(windows::Win32::UI::WindowsAndMessaging::SetWindowLongA(
                hwnd,
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as i32,
            ))
        };

        wnd_proc
    });

    render(hwnd);

    trace!("Call IDXGISwapChain::Present trampoline");
    dxgi_swap_chain_present(p_this, sync_interval, flags)
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

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    let render_engine = match RENDER_ENGINE.get().map(Mutex::try_lock) {
        Some(Some(render_engine)) => render_engine,
        Some(None) => {
            debug!("Could not lock in WndProc");
            return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
        },
        None => {
            debug!("WndProc called before hook was set");
            return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
        },
    };

    let Some(render_loop) = RENDER_LOOP.get() else {
        debug!("Could not get render loop");
        return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
    };

    let Some(&wnd_proc) = WND_PROC.get() else {
        debug!("Could not get original WndProc");
        return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
    };

    imgui_wnd_proc_impl(
        hwnd,
        umsg,
        WPARAM(wparam),
        LPARAM(lparam),
        wnd_proc,
        render_engine,
        render_loop,
    )
}

unsafe fn print_dxgi_debug_messages() {
    let diq: IDXGIInfoQueue = DXGIGetDebugInterface1(0).unwrap();

    for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
        let mut msg_len: usize = 0;
        diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap();
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _).unwrap();
        let diqm = pdiqm.as_ref().unwrap();
        debug!(
            "[DIQ] {}",
            String::from_utf8_lossy(std::slice::from_raw_parts(
                diqm.pDescription,
                diqm.DescriptionByteLength - 1
            ))
        );
    }
    diq.ClearStoredMessages(DXGI_DEBUG_ALL);
}

unsafe fn render(hwnd: HWND) {
    RENDER_LOCK.store(true, Ordering::SeqCst);

    let render_engine =
        RENDER_ENGINE.get_or_init(move || Mutex::new(RenderEngine::new(hwnd).unwrap()));

    let Some(mut render_engine) = render_engine.try_lock() else {
        error!("Could not lock render engine");
        return;
    };
    let Some(render_loop) = RENDER_LOOP.get_mut() else {
        error!("Could not obtain render loop");
        return;
    };

    if let Err(e) = render_engine.render(|ui| render_loop.render(ui)) {
        error!("Render: {e:?}");
    }

    RENDER_LOCK.store(false, Ordering::SeqCst);
}

fn get_target_addrs() -> (DXGISwapChainPresentType, DXGISwapChainResizeBuffersType) {
    // let dummy_hwnd = find_process_hwnd().expect("Could not find process hwnd"); // unsafe { GetDesktopWindow() };

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

    (present_ptr, resize_buffers_ptr)
}

pub struct ImguiDx12Hooks([MhHook; 2]);

impl ImguiDx12Hooks {
    /// Construct a set of [`RawDetour`]s that will render UI via the provided
    /// [`ImguiRenderLoop`].
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
        let (dxgi_swap_chain_present_addr, dxgi_swap_chain_resize_buffers_addr) =
            get_target_addrs();

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

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dxgi_swap_chain_present: mem::transmute(hook_present.trampoline()),
            dxgi_swap_chain_resize_buffers: mem::transmute(hook_resize_buffers.trampoline()),
        });

        Self([hook_present, hook_resize_buffers])
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
        RENDER_ENGINE.take();
        RENDER_LOOP.take();
        TRAMPOLINES.take();
        RENDER_LOCK.store(false, Ordering::SeqCst);
    }
}

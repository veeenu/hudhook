use std::{
    ffi::c_void,
    mem,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use tracing::{error, info, trace};
use windows::{
    core::{Interface, HRESULT},
    Win32::{
        Foundation::{BOOL, HWND},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext,
                D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
                    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
                },
                DXGIGetDebugInterface1, IDXGIInfoQueue, IDXGISwapChain, DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
        UI::WindowsAndMessaging::GetDesktopWindow,
    },
};

use crate::mh::MhHook;
use crate::renderer::dx12::RenderEngine;
use crate::Hooks;
use crate::ImguiRenderLoop;

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain, SyncInterval: u32, Flags: u32) -> HRESULT;

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut RENDER_ENGINE: OnceLock<Mutex<RenderEngine>> = OnceLock::new();
static mut RENDER_LOOP: OnceLock<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceLock::new();
static mut TRAMPOLINE: OnceLock<DXGISwapChainPresentType> = OnceLock::new();

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let trampoline = TRAMPOLINE.get().expect("IDXGISwapChain::Present trampoline uninitialized");
    let hwnd = GAME_HWND.get_or_init(|| {
        let mut desc = Default::default();
        p_this.GetDesc(&mut desc).unwrap();
        info!("Output window: {:?}", p_this);
        info!("Desc: {:?}", desc);
        desc.OutputWindow
    });

    render(*hwnd);

    trace!("Call trampoline");
    trampoline(p_this, sync_interval, flags)
}

unsafe fn render(hwnd: HWND) {
    let render_engine =
        RENDER_ENGINE.get_or_init(move || Mutex::new(RenderEngine::new(hwnd).unwrap()));

    let Ok(mut render_engine) = render_engine.try_lock() else { return };
    let Some(render_loop) = RENDER_LOOP.get_mut() else { return };

    if let Err(e) = render_engine.render(|ui| render_loop.render(ui)) {
        error!("Render: {e:?}");
    }
}

fn get_present_addr() -> DXGISwapChainPresentType {
    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    let dummy_hwnd = unsafe { GetDesktopWindow() };
    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_NULL,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&[D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                    ..Default::default()
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 1,
                OutputWindow: dummy_hwnd,
                Windowed: BOOL(1),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, ..Default::default() },
                ..Default::default()
            }),
            Some(&mut p_swap_chain),
            Some(&mut p_device),
            None,
            Some(&mut p_context),
        )
        .expect("D3D11CreateDeviceAndSwapChain failed");
    }

    let swap_chain = p_swap_chain.unwrap();

    let present_ptr = swap_chain.vtable().Present;

    unsafe { std::mem::transmute(present_ptr) }
}

pub struct ImguiDx11Hooks([MhHook; 1]);

impl ImguiDx11Hooks {
    /// Construct a set of [`RawDetour`]s that will render UI via the provided
    /// [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain::Present`
    /// - `IDXGISwapChain::ResizeBuffers`
    /// - `ID3D12CommandQueue::ExecuteCommandLists`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let dxgi_swap_chain_present_addr = get_present_addr();

        trace!("IDXGISwapChain::Present = {:p}", dxgi_swap_chain_present_addr as *const c_void);
        let hook_dscp = MhHook::new(
            dxgi_swap_chain_present_addr as *mut _,
            dxgi_swap_chain_present_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::Present hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| mem::transmute(hook_dscp.trampoline()));

        Self([hook_dscp])
    }
}

impl Hooks for ImguiDx11Hooks {
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
        trace!("Disabling hooks...");

        // CQECL_RUNNING.wait();
        // PRESENT_RUNNING.wait();
        // RBUF_RUNNING.wait();

        trace!("Cleaning up renderer...");
        if let Some(_renderer) = RENDER_ENGINE.take() {
            // let renderer = renderer.lock().unwrap();
            // XXX
            // This is a hack for solving this concurrency issue:
            // https://github.com/veeenu/hudhook/issues/34
            // We should investigate deeper into this and find a way of synchronizing with
            // the moment the actual resources involved in the rendering are
            // dropped. Using a condvar like above does not work, and still
            // leads clients to crash.
            //
            // The 34ms value was chosen because it's a bit more than 1 frame @ 30fps.
            thread::sleep(Duration::from_millis(34));
            // renderer.cleanup(None);
        }

        drop(RENDER_LOOP.take());
        // COMMAND_QUEUE_GUARD.take();
        //
        // DXGI_DEBUG_ENABLED.store(false, Ordering::SeqCst);
    }
}

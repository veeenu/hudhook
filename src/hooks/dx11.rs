use std::ffi::c_void;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, HRESULT};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW};

use super::DummyHwnd;
use crate::compositor::dx11::Compositor;
use crate::mh::MhHook;
use crate::pipeline::{Pipeline, PipelineMessage, PipelineSharedState};
use crate::renderer::print_dxgi_debug_messages;
use crate::{util, Hooks, ImguiRenderLoop};

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain, SyncInterval: u32, Flags: u32) -> HRESULT;

struct Trampolines {
    dxgi_swap_chain_present: DXGISwapChainPresentType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> =
    OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(
    swap_chain: &IDXGISwapChain,
) -> Result<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> {
    let hwnd = util::try_out_param(|v| swap_chain.GetDesc(v)).map(|desc| desc.OutputWindow)?;
    let compositor = Compositor::new(&swap_chain.GetDevice()?)?;

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

fn render(swap_chain: &IDXGISwapChain) -> Result<()> {
    let (pipeline, _) = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(swap_chain)) }?;

    let Some(mut pipeline) = pipeline.try_lock() else {
        return Err(Error::new(HRESULT(-1), "Could not lock pipeline".into()));
    };

    let source = pipeline.render()?;
    let handle = pipeline.engine_mut().create_shared_handle(source)?;

    pipeline.compositor_mut().composite(handle, swap_chain)?;

    Ok(())
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(shared_state) = PIPELINE.get().map(|(_, shared_state)| shared_state) else {
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
    swap_chain: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_present } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");

    if let Err(e) = render(&swap_chain) {
        print_dxgi_debug_messages();
        error!("Render error: {e:?}");
    }

    trace!("Call IDXGISwapChain::Present trampoline");
    dxgi_swap_chain_present(swap_chain, sync_interval, flags)
}

fn get_target_addrs() -> DXGISwapChainPresentType {
    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    let dummy_hwnd = DummyHwnd::new();
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
                OutputWindow: dummy_hwnd.hwnd(),
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

    let present_ptr: DXGISwapChainPresentType =
        unsafe { mem::transmute(swap_chain.vtable().Present) };

    present_ptr
}

pub struct ImguiDx11Hooks([MhHook; 1]);

impl ImguiDx11Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain::Present`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let dxgi_swap_chain_present_addr = get_target_addrs();

        trace!("IDXGISwapChain::Present = {:p}", dxgi_swap_chain_present_addr as *const c_void);
        let hook_present = MhHook::new(
            dxgi_swap_chain_present_addr as *mut _,
            dxgi_swap_chain_present_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::Present hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dxgi_swap_chain_present: mem::transmute(hook_present.trampoline()),
        });

        Self([hook_present])
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
        TRAMPOLINES.take();
        PIPELINE.take();
        RENDER_LOOP.take(); // should already be null
    }
}

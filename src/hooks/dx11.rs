//! Hooks for DirectX 11.

use std::cell::Cell;
use std::ffi::c_void;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::{Mutex, ReentrantMutex};
use tracing::{error, trace};
use windows::core::{Error, IUnknown, Interface, Result, BOOL, HRESULT};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, IDXGISwapChain1, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_PRESENT, DXGI_PRESENT_PARAMETERS, DXGI_PRESENT_TEST, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use super::{dxgi, DummyHwnd};
use crate::mh::MhHook;
use crate::renderer::{D3D11RenderEngine, Pipeline};
use crate::{perform_eject, Hooks, ImguiRenderLoop, EJECT_REQUESTED, HOOK_EJECTION_BARRIER};

type DXGISwapChainPresentType =
    unsafe extern "system" fn(this: IDXGISwapChain, sync_interval: u32, flags: u32) -> HRESULT;

type DXGISwapChainPresent1Type = unsafe extern "system" fn(
    this: IDXGISwapChain1,
    sync_interval: u32,
    flags: u32,
    parameters: *const DXGI_PRESENT_PARAMETERS,
) -> HRESULT;

type DXGISwapChainResizeBuffersType = unsafe extern "system" fn(
    this: IDXGISwapChain,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT;

struct Trampolines {
    dxgi_swap_chain_present: DXGISwapChainPresentType,
    dxgi_swap_chain_present1: DXGISwapChainPresent1Type,
    dxgi_swap_chain_resize_buffers: DXGISwapChainResizeBuffersType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<Mutex<Pipeline<D3D11RenderEngine>>> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static PIPELINE_LOCK: ReentrantMutex<()> = ReentrantMutex::new(());
static ACTIVE_CONTEXT: Mutex<Option<(usize, usize)>> = Mutex::new(None);

thread_local! {
    static IN_PRESENT: Cell<bool> = const { Cell::new(false) };
}

struct PresentGuard;

impl PresentGuard {
    fn enter() -> Option<Self> {
        IN_PRESENT.with(|active| if active.replace(true) { None } else { Some(Self) })
    }
}

impl Drop for PresentGuard {
    fn drop(&mut self) {
        IN_PRESENT.with(|active| active.set(false));
    }
}

unsafe fn reset_pipeline() {
    if let Some(pipeline) = PIPELINE.take() {
        RENDER_LOOP.set(pipeline.into_inner().take()).ok();
    }
    *ACTIVE_CONTEXT.lock() = None;
}

unsafe fn init_pipeline(swap_chain: &IDXGISwapChain) -> Result<Mutex<Pipeline<D3D11RenderEngine>>> {
    let desc = swap_chain.GetDesc()?;
    let hwnd = desc.OutputWindow;
    let device: ID3D11Device = swap_chain.GetDevice()?;
    let device_identity = device.cast::<IUnknown>()?.as_raw() as usize;
    let display_size = dxgi::display_size(swap_chain)?;

    let mut ctx = Context::create();
    let engine = D3D11RenderEngine::new(&device, &mut ctx)?;

    let Some(render_loop) = RENDER_LOOP.take() else {
        error!("Render loop not yet initialized");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    let pipeline = Pipeline::new(hwnd, ctx, engine, render_loop, display_size).map_err(
        |(e, render_loop)| {
            RENDER_LOOP.get_or_init(move || render_loop);
            e
        },
    )?;
    *ACTIVE_CONTEXT.lock() = Some((device_identity, hwnd.0 as usize));

    Ok(Mutex::new(pipeline))
}

fn render(swap_chain: &IDXGISwapChain) -> Result<()> {
    unsafe {
        let device: ID3D11Device = swap_chain.GetDevice()?;
        device.GetDeviceRemovedReason()?;
        let context = (
            device.cast::<IUnknown>()?.as_raw() as usize,
            swap_chain.GetDesc()?.OutputWindow.0 as usize,
        );
        if ACTIVE_CONTEXT.lock().is_some_and(|active| active != context) {
            reset_pipeline();
        }
        let pipeline = PIPELINE.get_or_try_init(|| init_pipeline(swap_chain))?;

        let Some(mut pipeline) = pipeline.try_lock() else {
            error!("Could not lock pipeline");
            return Err(Error::from_hresult(HRESULT(-1)));
        };

        pipeline.prepare_render(Some(dxgi::display_size(swap_chain)?))?;

        let target: ID3D11Texture2D = swap_chain.GetBuffer(0)?;

        pipeline.render(target)?;
    }
    Ok(())
}

unsafe fn present(
    swap_chain: &IDXGISwapChain,
    flags: u32,
    trampoline: impl FnOnce() -> HRESULT,
) -> HRESULT {
    let Some(present_guard) = PresentGuard::enter() else {
        return trampoline();
    };
    if flags & DXGI_PRESENT_TEST.0 == 0 {
        if let Some(_pipeline_guard) = PIPELINE_LOCK.try_lock() {
            if let Err(error) = render(swap_chain) {
                if matches!(error.code(), DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET) {
                    reset_pipeline();
                }
                error!("Render error: {error:?}");
            }
        }
    }
    // DXGI may wait for the window thread, which can call ResizeBuffers.
    let result = trampoline();
    if matches!(result, DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET) {
        let _pipeline_guard = PIPELINE_LOCK.lock();
        reset_pipeline();
    }
    if EJECT_REQUESTED.load(Ordering::SeqCst) {
        drop(present_guard);
        perform_eject();
    }
    result
}

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    swap_chain: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();
    let Trampolines { dxgi_swap_chain_present, .. } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");
    present(&swap_chain, flags, || {
        dxgi_swap_chain_present(swap_chain.clone(), sync_interval, flags)
    })
}

unsafe extern "system" fn dxgi_swap_chain_present1_impl(
    swap_chain: IDXGISwapChain1,
    sync_interval: u32,
    flags: u32,
    parameters: *const DXGI_PRESENT_PARAMETERS,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();
    let Trampolines { dxgi_swap_chain_present1, .. } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");
    present(&swap_chain, flags, || {
        dxgi_swap_chain_present1(swap_chain.clone(), sync_interval, flags, parameters)
    })
}

unsafe extern "system" fn dxgi_swap_chain_resize_buffers_impl(
    swap_chain: IDXGISwapChain,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();
    let _pipeline_guard = PIPELINE_LOCK.lock();

    let Trampolines { dxgi_swap_chain_resize_buffers, .. } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");

    trace!("Call IDXGISwapChain::ResizeBuffers trampoline");
    dxgi_swap_chain_resize_buffers(swap_chain, buffer_count, width, height, new_format, flags)
}

fn get_target_addrs(
) -> (DXGISwapChainPresentType, DXGISwapChainPresent1Type, DXGISwapChainResizeBuffersType) {
    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    let dummy_hwnd = DummyHwnd::new();
    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE(std::ptr::null_mut()),
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

    let present_ptr: DXGISwapChainPresentType = unsafe {
        mem::transmute::<
            unsafe extern "system" fn(*mut c_void, u32, DXGI_PRESENT) -> HRESULT,
            DXGISwapChainPresentType,
        >(swap_chain.vtable().Present)
    };

    let resize_buffers_ptr: DXGISwapChainResizeBuffersType = unsafe {
        mem::transmute::<*mut c_void, DXGISwapChainResizeBuffersType>(
            swap_chain.vtable().ResizeBuffers as *mut c_void,
        )
    };

    let swap_chain1: IDXGISwapChain1 = swap_chain.cast().unwrap();
    let present1_ptr: DXGISwapChainPresent1Type = unsafe {
        mem::transmute::<*mut c_void, DXGISwapChainPresent1Type>(
            swap_chain1.vtable().Present1 as *mut c_void,
        )
    };

    (present_ptr, present1_ptr, resize_buffers_ptr)
}

/// Hooks for DirectX 11.
pub struct ImguiDx11Hooks([MhHook; 3]);

impl ImguiDx11Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain::Present`
    /// - `IDXGISwapChain1::Present1`
    /// - `IDXGISwapChain::ResizeBuffers`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        let (
            dxgi_swap_chain_present_addr,
            dxgi_swap_chain_present1_addr,
            dxgi_swap_chain_resize_buffers_addr,
        ) = get_target_addrs();

        trace!("IDXGISwapChain::Present = {:p}", dxgi_swap_chain_present_addr as *const c_void);
        let hook_present = MhHook::new(
            dxgi_swap_chain_present_addr as *mut _,
            dxgi_swap_chain_present_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::Present hook");

        trace!("IDXGISwapChain1::Present1 = {:p}", dxgi_swap_chain_present1_addr as *const c_void);
        let hook_present1 = MhHook::new(
            dxgi_swap_chain_present1_addr as *mut _,
            dxgi_swap_chain_present1_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain1::Present1 hook");

        trace!(
            "IDXGISwapChain::ResizeBuffers = {:p}",
            dxgi_swap_chain_resize_buffers_addr as *const c_void
        );
        let hook_resize_buffers = MhHook::new(
            dxgi_swap_chain_resize_buffers_addr as *mut _,
            dxgi_swap_chain_resize_buffers_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::ResizeBuffers hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dxgi_swap_chain_present: mem::transmute::<*mut c_void, DXGISwapChainPresentType>(
                hook_present.trampoline(),
            ),
            dxgi_swap_chain_present1: mem::transmute::<*mut c_void, DXGISwapChainPresent1Type>(
                hook_present1.trampoline(),
            ),
            dxgi_swap_chain_resize_buffers: mem::transmute::<
                *mut c_void,
                DXGISwapChainResizeBuffersType,
            >(hook_resize_buffers.trampoline()),
        });

        Self([hook_present, hook_present1, hook_resize_buffers])
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
        let _pipeline_guard = PIPELINE_LOCK.lock();
        TRAMPOLINES.take();
        PIPELINE.take().map(|p| p.into_inner().take());
        RENDER_LOOP.take(); // should already be null
        *ACTIVE_CONTEXT.lock() = None;
    }
}

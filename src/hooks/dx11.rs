use std::{ffi::c_void, mem, sync::OnceLock};

use parking_lot::Mutex;
use tracing::{debug, error, info, trace};
use windows::{
    core::{Interface, HRESULT},
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext,
                D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC,
                    DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    DXGI_SAMPLE_DESC,
                },
                IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
        UI::WindowsAndMessaging::{DefWindowProcW, GetDesktopWindow, GWLP_WNDPROC},
    },
};

use crate::mh::MhHook;
use crate::renderer::dx12::RenderEngine;
use crate::Hooks;
use crate::ImguiRenderLoop;

use super::input::{imgui_wnd_proc_impl, WndProcType};

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain, SyncInterval: u32, Flags: u32) -> HRESULT;

type DXGISwapChainResizeBuffersType = unsafe extern "system" fn(
    This: IDXGISwapChain,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT;

struct Trampolines {
    dxgi_swap_chain_present: DXGISwapChainPresentType,
    dxgi_swap_chain_resize_buffers: DXGISwapChainResizeBuffersType,
}

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut WND_PROC: OnceLock<WndProcType> = OnceLock::new();
static mut RENDER_ENGINE: OnceLock<Mutex<RenderEngine>> = OnceLock::new();
static mut RENDER_LOOP: OnceLock<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceLock::new();
static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_present, .. } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");
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
    p_this: IDXGISwapChain,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    let Trampolines { dxgi_swap_chain_resize_buffers, .. } =
        TRAMPOLINES.get().expect("DirectX 11 trampolines uninitialized");

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

unsafe fn render(hwnd: HWND) {
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
}

fn get_target_addrs() -> (DXGISwapChainPresentType, DXGISwapChainResizeBuffersType) {
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

    let present_ptr: DXGISwapChainPresentType =
        unsafe { mem::transmute(swap_chain.vtable().Present) };
    let resize_buffers_ptr: DXGISwapChainResizeBuffersType =
        unsafe { mem::transmute(swap_chain.vtable().ResizeBuffers) };

    (present_ptr, resize_buffers_ptr)
}

pub struct ImguiDx11Hooks([MhHook; 2]);

impl ImguiDx11Hooks {
    /// Construct a set of [`RawDetour`]s that will render UI via the provided
    /// [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain::Present`
    /// - `IDXGISwapChain::ResizeBuffers`
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
        RENDER_ENGINE.take();
        RENDER_LOOP.take();
        TRAMPOLINES.take();
    }
}

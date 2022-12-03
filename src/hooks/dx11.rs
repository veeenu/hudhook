use std::ffi::c_void;
use std::mem;
use std::ptr::null_mut;

use imgui::Context;
use log::*;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::{
    GetLastError, BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::common::{
    imgui_wnd_proc_impl, ImguiRenderLoop, ImguiRenderLoopFlags, ImguiWindowsEventHandler,
};
use super::Hooks;
use crate::mh::{MhHook, MhHooks};
use crate::renderers::imgui_dx11;

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

type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Data structures and traits
////////////////////////////////////////////////////////////////////////////////////////////////////

trait Renderer {
    /// Invoked once per frame.
    fn render(&mut self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global singletons
////////////////////////////////////////////////////////////////////////////////////////////////////

static TRAMPOLINE: OnceCell<(DXGISwapChainPresentType, DXGISwapChainResizeBuffersType)> =
    OnceCell::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hook entry points
////////////////////////////////////////////////////////////////////////////////////////////////////

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static mut IMGUI_RENDERER: OnceCell<Mutex<Box<ImguiRenderer>>> = OnceCell::new();

unsafe extern "system" fn imgui_dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let (trampoline, _) =
        TRAMPOLINE.get().expect("IDXGISwapChain::Present trampoline uninitialized");

    let mut renderer = IMGUI_RENDERER
        .get_or_init(|| Mutex::new(Box::new(ImguiRenderer::new(p_this.clone()))))
        .lock();

    renderer.render(Some(p_this.clone()));
    drop(renderer);

    trace!("Invoking IDXGISwapChain::Present trampoline");
    let r = trampoline(p_this, sync_interval, flags);
    trace!("Trampoline returned {:?}", r);

    r
}

unsafe extern "system" fn imgui_resize_buffers_impl(
    swap_chain: IDXGISwapChain,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    trace!("IDXGISwapChain::ResizeBuffers invoked");
    let (_, trampoline) =
        TRAMPOLINE.get().expect("IDXGISwapChain::ResizeBuffer trampoline uninitialized");

    if let Some(mutex) = IMGUI_RENDERER.take() {
        mutex.lock().cleanup(Some(swap_chain.clone()));
    };

    trampoline(swap_chain, buffer_count, width, height, new_format, flags)
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    match IMGUI_RENDERER.get().map(Mutex::try_lock) {
        Some(Some(imgui_renderer)) => imgui_wnd_proc_impl(
            hwnd,
            umsg,
            WPARAM(wparam),
            LPARAM(lparam),
            imgui_renderer,
            IMGUI_RENDER_LOOP.get().unwrap(),
        ),
        Some(None) => {
            debug!("Could not lock in WndProc");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        },
        None => {
            debug!("WndProc called before hook was set");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        },
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Render loops
////////////////////////////////////////////////////////////////////////////////////////////////////

struct ImguiRenderer {
    ctx: Context,
    engine: imgui_dx11::RenderEngine,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
    swap_chain: IDXGISwapChain,
}

impl ImguiRenderer {
    unsafe fn new(swap_chain: IDXGISwapChain) -> Self {
        trace!("Initializing imgui context");

        let mut ctx = Context::create();
        ctx.set_ini_filename(None);

        IMGUI_RENDER_LOOP.get_mut().unwrap().initialize(&mut ctx);

        let flags = ImguiRenderLoopFlags { focused: true };

        trace!("Initializing renderer");

        let dev: ID3D11Device = swap_chain.GetDevice().expect("GetDevice");
        let mut dev_ctx: Option<ID3D11DeviceContext> = None;
        dev.GetImmediateContext(&mut dev_ctx);
        let dev_ctx = dev_ctx.unwrap();
        let sd = swap_chain.GetDesc().expect("GetDesc");

        let engine =
            imgui_dx11::RenderEngine::new_with_ptrs(dev, dev_ctx, swap_chain.clone(), &mut ctx);

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongPtrA(
            sd.OutputWindow,
            GWLP_WNDPROC,
            imgui_wnd_proc as usize as isize,
        ));

        #[cfg(target_arch = "x86")]
        let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongA(
            sd.OutputWindow,
            GWLP_WNDPROC,
            imgui_wnd_proc as usize as i32,
        ));

        trace!("Renderer initialized");
        let mut renderer = ImguiRenderer { ctx, engine, wnd_proc, flags, swap_chain };

        ImguiWindowsEventHandler::setup_io(&mut renderer);

        renderer
    }

    unsafe fn render(&mut self, swap_chain: Option<IDXGISwapChain>) {
        trace!("Present impl: Rendering");

        let swap_chain = self.store_swap_chain(swap_chain);
        let sd = swap_chain.GetDesc().expect("GetDesc");

        // if GetWindowRect(sd.OutputWindow, &mut rect as _).as_bool() {
        if let Some(rect) = self.engine.get_window_rect() {
            let mut io = self.ctx_mut().io_mut();

            io.display_size = [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];

            let mut pos = POINT { x: 0, y: 0 };

            let active_window = GetForegroundWindow();
            if !HANDLE(active_window.0).is_invalid()
                && (active_window == sd.OutputWindow
                    || IsChild(active_window, sd.OutputWindow).as_bool())
            {
                let gcp = GetCursorPos(&mut pos as *mut _);
                if gcp.as_bool() && ScreenToClient(sd.OutputWindow, &mut pos as *mut _).as_bool() {
                    io.mouse_pos[0] = pos.x as _;
                    io.mouse_pos[1] = pos.y as _;
                }
            }
        } else {
            trace!("GetWindowRect error: {:x}", GetLastError().0);
        }

        let ctx = &mut self.ctx;
        let mut ui = ctx.frame();
        IMGUI_RENDER_LOOP.get_mut().unwrap().render(&mut ui, &self.flags);
        let draw_data = ctx.render();

        if let Err(e) = self.engine.render_draw_data(draw_data) {
            // if let Err(e) = self.engine.render(|ui| self.render_loop.render(ui,
            // &self.flags)) {
            error!("ImGui renderer error: {:?}", e);
        }
    }

    fn store_swap_chain(&mut self, swap_chain: Option<IDXGISwapChain>) -> IDXGISwapChain {
        if let Some(swap_chain) = swap_chain {
            self.swap_chain = swap_chain;
        }

        self.swap_chain.clone()
    }

    unsafe fn cleanup(&mut self, swap_chain: Option<IDXGISwapChain>) {
        let swap_chain = self.store_swap_chain(swap_chain);
        let desc = swap_chain.GetDesc().unwrap();

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        SetWindowLongPtrA(desc.OutputWindow, GWLP_WNDPROC, self.wnd_proc as usize as isize);

        #[cfg(target_arch = "x86")]
        SetWindowLongA(desc.OutputWindow, GWLP_WNDPROC, self.wnd_proc as usize as i32);
    }

    fn ctx(&self) -> &imgui::Context {
        &self.ctx
    }

    fn ctx_mut(&mut self) -> &mut imgui::Context {
        &mut self.ctx
    }
}

impl ImguiWindowsEventHandler for ImguiRenderer {
    fn io(&self) -> &imgui::Io {
        self.ctx().io()
    }

    fn io_mut(&mut self) -> &mut imgui::Io {
        self.ctx_mut().io_mut()
    }

    fn focus(&self) -> bool {
        self.flags.focused
    }

    fn focus_mut(&mut self) -> &mut bool {
        &mut self.flags.focused
    }

    fn wnd_proc(&self) -> WndProcType {
        self.wnd_proc
    }
}

unsafe impl Send for ImguiRenderer {}
unsafe impl Sync for ImguiRenderer {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.
fn get_present_addr() -> (DXGISwapChainPresentType, DXGISwapChainResizeBuffersType) {
    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            &[D3D_FEATURE_LEVEL_11_0],
            D3D11_SDK_VERSION,
            &DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                    ..Default::default()
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 1,
                OutputWindow: GetDesktopWindow(),
                Windowed: BOOL(1),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, ..Default::default() },
                ..Default::default()
            },
            &mut p_swap_chain,
            &mut p_device,
            null_mut(),
            &mut p_context,
        )
        .expect("D3D11CreateDeviceAndSwapChain failed");
    }

    let swap_chain = p_swap_chain.unwrap();

    let present_ptr = swap_chain.vtable().Present;
    let resize_buffers_ptr = swap_chain.vtable().ResizeBuffers;

    unsafe { (std::mem::transmute(present_ptr), std::mem::transmute(resize_buffers_ptr)) }
}

pub struct ImguiDx11Hooks(MhHooks);

impl ImguiDx11Hooks {
    /// Construct a [`RawDetour`] that will render UI via the provided
    /// `ImguiRenderLoop`.
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: Send + Sync + ImguiRenderLoop,
    {
        let (present_addr, resize_buffers_addr) = get_present_addr();
        debug!("IDXGISwapChain::Present = {:p}", present_addr as *mut c_void);
        debug!("IDXGISwapChain::ResizeBuffers = {:p}", resize_buffers_addr as *mut c_void);

        let hook_present =
            MhHook::new(present_addr as *mut _, imgui_dxgi_swap_chain_present_impl as *mut _)
                .expect("couldn't create IDXGISwapChain::Present hook");
        let hook_resize_buffers =
            MhHook::new(resize_buffers_addr as *mut _, imgui_resize_buffers_impl as *mut _)
                .expect("couldn't create IDXGISwapChain::ResizeBuffers hook");

        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| {
            (
                mem::transmute(hook_present.trampoline()),
                mem::transmute(hook_resize_buffers.trampoline()),
            )
        });

        Self(MhHooks::new([hook_present, hook_resize_buffers]).expect("couldn't create hooks"))
    }
}

impl Hooks for ImguiDx11Hooks {
    unsafe fn hook(&self) {
        self.0.apply();
    }

    unsafe fn unhook(&mut self) {
        trace!("Disabling hooks...");
        self.0.unapply();

        trace!("Cleaning up renderer...");
        if let Some(renderer) = IMGUI_RENDERER.take() {
            renderer.lock().cleanup(None);
        }

        drop(IMGUI_RENDER_LOOP.take());
    }
}

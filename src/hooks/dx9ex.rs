//! Hooks for DirectX 9Ex.

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::{mem, ptr};

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9Ex, IDirect3DDevice9Ex, D3DADAPTER_DEFAULT, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DDISPLAYMODEEX,
    D3DFORMAT, D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::RGNDATA;

use super::DummyHwnd;
use crate::mh::MhHook;
use crate::renderer::{D3D9RenderEngine, Pipeline};
use crate::{perform_eject, util, Hooks, ImguiRenderLoop, EJECT_REQUESTED, HOOK_EJECTION_BARRIER};

type Dx9ExPresentType = unsafe extern "system" fn(
    this: IDirect3DDevice9Ex,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

type Dx9ExPresentExType = unsafe extern "system" fn(
    this: IDirect3DDevice9Ex,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
    dwflags: u32,
) -> HRESULT;

type Dx9ExResetType =
    unsafe extern "system" fn(this: IDirect3DDevice9Ex, *const D3DPRESENT_PARAMETERS) -> HRESULT;

type Dx9ExResetExType = unsafe extern "system" fn(
    this: IDirect3DDevice9Ex,
    *const D3DPRESENT_PARAMETERS,
    *const D3DDISPLAYMODEEX,
) -> HRESULT;

struct Trampolines {
    dx9ex_present: Dx9ExPresentType,
    dx9ex_present_ex: Dx9ExPresentExType,
    dx9ex_reset: Dx9ExResetType,
    dx9ex_reset_ex: Dx9ExResetExType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<Mutex<Pipeline<D3D9RenderEngine>>> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(device: &IDirect3DDevice9Ex) -> Result<Mutex<Pipeline<D3D9RenderEngine>>> {
    trace!("initializing pipeline");
    let mut creation_parameters = Default::default();
    device.GetCreationParameters(&mut creation_parameters)?;

    let hwnd = creation_parameters.hFocusWindow;

    let mut ctx = Context::create();
    trace!("creating engine");
    let engine = D3D9RenderEngine::new(device, &mut ctx)?;

    let Some(render_loop) = RENDER_LOOP.take() else {
        error!("Render loop not yet initialized");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    trace!("creating pipeline");
    let pipeline = Pipeline::new(hwnd, ctx, engine, render_loop).map_err(|(e, render_loop)| {
        RENDER_LOOP.get_or_init(move || render_loop);
        e
    })?;
    Ok(Mutex::new(pipeline))
}

fn render(device: &IDirect3DDevice9Ex) -> Result<()> {
    let pipeline = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(device)) }?;

    let Some(mut pipeline) = pipeline.try_lock() else {
        error!("Could not lock pipeline");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    pipeline.prepare_render()?;

    let surface = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO)? };

    unsafe { device.BeginScene() }?;
    let render_result = pipeline.render(surface);
    unsafe { device.EndScene() }?;

    render_result
}

unsafe fn reset_pipeline() {
    trace!("Resetting pipeline");
    if let Some(pipeline) = PIPELINE.take() {
        let render_loop = pipeline.into_inner().take();

        RENDER_LOOP.set(render_loop).map_err(|_| ()).expect("Render loop cell should be empty");
    }
}

unsafe extern "system" fn dx9ex_present_impl(
    device: IDirect3DDevice9Ex,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9ex_present, .. } =
        TRAMPOLINES.get().expect("DirectX 9Ex trampolines uninitialized");

    if let Err(e) = render(&device) {
        error!("Render error: {e:?}");
    }

    trace!("Call IDirect3DDevice9Ex::Present trampoline");
    let result = dx9ex_present(device, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion);
    if EJECT_REQUESTED.load(Ordering::SeqCst) {
        perform_eject();
    }
    result
}

unsafe extern "system" fn dx9ex_present_ex_impl(
    device: IDirect3DDevice9Ex,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
    dwflags: u32,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9ex_present_ex, .. } =
        TRAMPOLINES.get().expect("DirectX 9Ex trampolines uninitialized");

    if let Err(e) = render(&device) {
        error!("Render error: {e:?}");
    }

    trace!("Call IDirect3DDevice9Ex::PresentEx trampoline");
    let result = dx9ex_present_ex(
        device,
        psourcerect,
        pdestrect,
        hdestwindowoverride,
        pdirtyregion,
        dwflags,
    );
    if EJECT_REQUESTED.load(Ordering::SeqCst) {
        perform_eject();
    }
    result
}

unsafe extern "system" fn dx9ex_reset_impl(
    this: IDirect3DDevice9Ex,
    present_params: *const D3DPRESENT_PARAMETERS,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9ex_reset, .. } =
        TRAMPOLINES.get().expect("DirectX 9Ex trampolines uninitialized");

    reset_pipeline();

    dx9ex_reset(this, present_params)
}

unsafe extern "system" fn dx9ex_reset_ex_impl(
    this: IDirect3DDevice9Ex,
    present_params: *const D3DPRESENT_PARAMETERS,
    fullscreen_display_mode: *const D3DDISPLAYMODEEX,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9ex_reset_ex, .. } =
        TRAMPOLINES.get().expect("DirectX 9Ex trampolines uninitialized");

    reset_pipeline();

    dx9ex_reset_ex(this, present_params, fullscreen_display_mode)
}

fn get_target_addrs() -> (Dx9ExPresentType, Dx9ExPresentExType, Dx9ExResetType, Dx9ExResetExType) {
    let d9 = unsafe { Direct3DCreate9Ex(D3D_SDK_VERSION).unwrap() };

    let mut d3d_display_mode =
        D3DDISPLAYMODE { Width: 0, Height: 0, RefreshRate: 0, Format: D3DFORMAT(0) };
    unsafe { d9.GetAdapterDisplayMode(D3DADAPTER_DEFAULT, &mut d3d_display_mode).unwrap() };

    let mut present_params = D3DPRESENT_PARAMETERS {
        Windowed: BOOL(1),
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        BackBufferFormat: d3d_display_mode.Format,
        ..Default::default()
    };

    let dummy_hwnd = DummyHwnd::new();
    let device: IDirect3DDevice9Ex = util::try_out_ptr(|v| unsafe {
        d9.CreateDeviceEx(
            D3DADAPTER_DEFAULT,
            D3DDEVTYPE_NULLREF,
            dummy_hwnd.hwnd(),
            D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
            &mut present_params,
            ptr::null_mut(),
            v,
        )
    })
    .expect("IDirect3D9Ex::CreateDeviceEx: failed to create device");

    let present_ptr = device.vtable().base__.Present;
    let present_ex_ptr = device.vtable().PresentEx;
    let reset_ptr = device.vtable().base__.Reset;
    let reset_ex_ptr = device.vtable().ResetEx;

    unsafe {
        (
            mem::transmute::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const RECT,
                    *const RECT,
                    HWND,
                    *const RGNDATA,
                ) -> HRESULT,
                Dx9ExPresentType,
            >(present_ptr),
            mem::transmute::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const RECT,
                    *const RECT,
                    HWND,
                    *const RGNDATA,
                    u32,
                ) -> HRESULT,
                Dx9ExPresentExType,
            >(present_ex_ptr),
            mem::transmute::<
                unsafe extern "system" fn(*mut c_void, *mut D3DPRESENT_PARAMETERS) -> HRESULT,
                Dx9ExResetType,
            >(reset_ptr),
            mem::transmute::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *mut D3DPRESENT_PARAMETERS,
                    *mut D3DDISPLAYMODEEX,
                ) -> HRESULT,
                Dx9ExResetExType,
            >(reset_ex_ptr),
        )
    }
}

/// Hooks for DirectX 9Ex.
pub struct ImguiDx9ExHooks([MhHook; 4]);

impl ImguiDx9ExHooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDirect3DDevice9Ex::Present`
    /// - `IDirect3DDevice9Ex::PresentEx`
    /// - `IDirect3DDevice9Ex::Reset`
    /// - `IDirect3DDevice9Ex::ResetEx`
    ///
    /// An application using DirectX9Ex may render through either the
    /// `Ex` functions or the ones inherited from `IDirect3DDevice9`, so both
    /// pairs are hooked.
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        let (dx9ex_present_addr, dx9ex_present_ex_addr, dx9ex_reset_addr, dx9ex_reset_ex_addr) =
            get_target_addrs();

        trace!("IDirect3DDevice9Ex::Present = {:p}", dx9ex_present_addr as *const c_void);
        trace!("IDirect3DDevice9Ex::PresentEx = {:p}", dx9ex_present_ex_addr as *const c_void);
        let hook_present =
            MhHook::new(dx9ex_present_addr as *mut c_void, dx9ex_present_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9Ex::Present hook");
        let hook_present_ex =
            MhHook::new(dx9ex_present_ex_addr as *mut c_void, dx9ex_present_ex_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9Ex::PresentEx hook");
        let hook_reset =
            MhHook::new(dx9ex_reset_addr as *mut c_void, dx9ex_reset_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9Ex::Reset hook");
        let hook_reset_ex =
            MhHook::new(dx9ex_reset_ex_addr as *mut c_void, dx9ex_reset_ex_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9Ex::ResetEx hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dx9ex_present: mem::transmute::<*mut c_void, Dx9ExPresentType>(
                hook_present.trampoline(),
            ),
            dx9ex_present_ex: mem::transmute::<*mut c_void, Dx9ExPresentExType>(
                hook_present_ex.trampoline(),
            ),
            dx9ex_reset: mem::transmute::<*mut c_void, Dx9ExResetType>(hook_reset.trampoline()),
            dx9ex_reset_ex: mem::transmute::<*mut c_void, Dx9ExResetExType>(
                hook_reset_ex.trampoline(),
            ),
        });

        Self([hook_present, hook_present_ex, hook_reset, hook_reset_ex])
    }
}

impl Hooks for ImguiDx9ExHooks {
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
        PIPELINE.take().map(|p| p.into_inner().take());
        RENDER_LOOP.take();
    }
}

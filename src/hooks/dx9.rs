//! Hooks for DirectX 9.

use std::ffi::c_void;
use std::mem;
use std::sync::OnceLock;

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, HRESULT};
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT,
    D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::RGNDATA;

use super::DummyHwnd;
use crate::mh::MhHook;
use crate::renderer::{D3D9RenderEngine, Pipeline};
use crate::{util, Hooks, ImguiRenderLoop};

type Dx9PresentType = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

type Dx9ResetType =
    unsafe extern "system" fn(this: IDirect3DDevice9, *const D3DPRESENT_PARAMETERS) -> HRESULT;

struct Trampolines {
    dx9_present: Dx9PresentType,
    dx9_reset: Dx9ResetType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<Mutex<Pipeline<D3D9RenderEngine>>> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(device: &IDirect3DDevice9) -> Result<Mutex<Pipeline<D3D9RenderEngine>>> {
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

fn render(device: &IDirect3DDevice9) -> Result<()> {
    let pipeline = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(device)) }?;

    let Some(mut pipeline) = pipeline.try_lock() else {
        error!("Could not lock pipeline");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    pipeline.prepare_render()?;

    let surface = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO)? };

    unsafe { device.BeginScene() }?;
    pipeline.render(surface)?;
    unsafe { device.EndScene() }?;

    Ok(())
}

unsafe extern "system" fn dx9_present_impl(
    device: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    let Trampolines { dx9_present, .. } =
        TRAMPOLINES.get().expect("DirectX 9 trampolines uninitialized");

    if let Err(e) = render(&device) {
        error!("Render error: {e:?}");
    }

    trace!("Call IDirect3DDevice9::Present trampoline");
    dx9_present(device, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion)
}
unsafe extern "system" fn dx9_reset_impl(
    this: IDirect3DDevice9,
    present_params: *const D3DPRESENT_PARAMETERS,
) -> HRESULT {
    let Trampolines { dx9_reset, .. } =
        TRAMPOLINES.get().expect("DirectX 9 trampolines uninitialized");

    trace!("Resetting pipeline");
    if let Some(pipeline) = PIPELINE.take() {
        let render_loop = pipeline.into_inner().take();

        RENDER_LOOP.set(render_loop).map_err(|_| ()).expect("Render loop cell should be empty");
    }

    dx9_reset(this, present_params)
}

fn get_target_addrs() -> (Dx9PresentType, Dx9ResetType) {
    let d9 = unsafe { Direct3DCreate9(D3D_SDK_VERSION).unwrap() };

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
    let device: IDirect3DDevice9 = util::try_out_ptr(|v| {
        unsafe {
            d9.CreateDevice(
                D3DADAPTER_DEFAULT,
                D3DDEVTYPE_NULLREF,
                dummy_hwnd.hwnd(), // GetDesktopWindow(),
                D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
                &mut present_params,
                v,
            )
        }
    })
    .expect("IDirect3DDevice9::CreateDevice: failed to create device");

    let present_ptr = device.vtable().Present;
    let reset_ptr = device.vtable().Reset;

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
                Dx9PresentType,
            >(present_ptr),
            mem::transmute::<
                unsafe extern "system" fn(*mut c_void, *mut D3DPRESENT_PARAMETERS) -> HRESULT,
                Dx9ResetType,
            >(reset_ptr),
        )
    }
}

/// Hooks for DirectX 9.
pub struct ImguiDx9Hooks([MhHook; 2]);

impl ImguiDx9Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDirect3DDevice9::Present`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        let (dx9_present_addr, dx9_reset_addr) = get_target_addrs();

        trace!("IDirect3DDevice9::Present = {:p}", dx9_present_addr as *const c_void);
        let hook_present =
            MhHook::new(dx9_present_addr as *mut c_void, dx9_present_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9::Present hook");
        let hook_reset = MhHook::new(dx9_reset_addr as *mut c_void, dx9_reset_impl as *mut c_void)
            .expect("couldn't create IDirect3DDevice9::Reset hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dx9_present: mem::transmute::<*mut c_void, Dx9PresentType>(hook_present.trampoline()),
            dx9_reset: mem::transmute::<*mut c_void, Dx9ResetType>(hook_reset.trampoline()),
        });

        Self([hook_present, hook_reset])
    }
}

impl Hooks for ImguiDx9Hooks {
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

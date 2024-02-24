use std::ffi::c_void;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, HRESULT};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DCREATE_SOFTWARE_VERTEXPROCESSING,
    D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT, D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD,
    D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::RGNDATA;
use windows::Win32::UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW};

use super::DummyHwnd;
use crate::compositor::dx9::Compositor;
use crate::mh::MhHook;
use crate::pipeline::{Pipeline, PipelineMessage, PipelineSharedState};
use crate::{util, Hooks, ImguiRenderLoop};

type Dx9PresentType = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

struct Trampolines {
    dx9_present: Dx9PresentType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> =
    OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(
    device: &IDirect3DDevice9,
) -> Result<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> {
    let mut creation_parameters = Default::default();
    let _ = device.GetCreationParameters(&mut creation_parameters);
    let hwnd = creation_parameters.hFocusWindow;

    let compositor = Compositor::new(device, hwnd)?;

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

fn render(device: &IDirect3DDevice9) -> Result<()> {
    let (pipeline, _) = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(device)) }?;

    let Some(mut pipeline) = pipeline.try_lock() else {
        return Err(Error::new(HRESULT(-1), "Could not lock pipeline".into()));
    };

    let source = pipeline.render()?;

    pipeline.compositor().composite(pipeline.engine(), source)?;

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

unsafe extern "system" fn dx9_present_impl(
    device: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    let Trampolines { dx9_present } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    if let Err(e) = render(&device) {
        util::print_dxgi_debug_messages();
        error!("Render error: {e:?}");
    }

    trace!("Call IDirect3DDevice9::Present trampoline");
    dx9_present(device, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion)
}

fn get_target_addrs() -> Dx9PresentType {
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

    unsafe { mem::transmute(present_ptr) }
}

pub struct ImguiDx9Hooks([MhHook; 1]);

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
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let dx9_present_addr = get_target_addrs();

        trace!("IDirect3DDevice9::Present = {:p}", dx9_present_addr as *const c_void);
        let hook_present = MhHook::new(dx9_present_addr as *mut _, dx9_present_impl as *mut _)
            .expect("couldn't create IDirect3DDevice9::Present hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES
            .get_or_init(|| Trampolines { dx9_present: mem::transmute(hook_present.trampoline()) });

        Self([hook_present])
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
        PIPELINE.take();
        RENDER_LOOP.take();
    }
}

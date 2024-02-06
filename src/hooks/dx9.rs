use std::ffi::c_void;
use std::mem;
use std::sync::OnceLock;

use tracing::trace;
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DCREATE_SOFTWARE_VERTEXPROCESSING,
    D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT, D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD,
    D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::RGNDATA;

use super::DummyHwnd;
use crate::mh::MhHook;
use crate::renderer::RenderState;
use crate::util::try_out_ptr;
use crate::{Hooks, ImguiRenderLoop};

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

unsafe extern "system" fn dx9_present_impl(
    p_this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    let Trampolines { dx9_present } =
        TRAMPOLINES.get().expect("DirectX 12 trampolines uninitialized");

    // Don't attempt a render if one is already underway: it might be that the
    // renderer itself is currently invoking `Present`.
    if RenderState::is_locked() {
        return dx9_present(p_this, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion);
    }

    let hwnd = RenderState::setup(|| {
        let mut creation_parameters = Default::default();
        let _ = p_this.GetCreationParameters(&mut creation_parameters);
        creation_parameters.hFocusWindow
    });

    RenderState::render(hwnd);

    trace!("Call IDirect3DDevice9::Present trampoline");
    dx9_present(p_this, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion)
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
    let device: IDirect3DDevice9 = try_out_ptr(|v| {
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

        RenderState::set_render_loop(t);
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
        RenderState::cleanup();
        TRAMPOLINES.take();
    }
}

use std::mem;
use std::sync::OnceLock;

use imgui::Context;
use parking_lot::Mutex;
use tracing::{debug, trace};
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::{
    GetLastError, BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT,
    D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::{ScreenToClient, RGNDATA};
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetDesktopWindow, GetForegroundWindow, IsChild, GWLP_WNDPROC,
};

use crate::hooks::common::{imgui_wnd_proc_impl, ImguiWindowsEventHandler, WndProcType};
use crate::hooks::{Hooks, ImguiRenderLoop, ImguiRenderLoopFlags};
use crate::mh::{MhHook, MhHooks};
use crate::renderers::imgui_dx9;

unsafe fn draw(this: &IDirect3DDevice9) {
    let mut imgui_renderer = IMGUI_RENDERER
        .get_or_init(|| {
            let mut context = imgui::Context::create();
            context.set_ini_filename(None);
            IMGUI_RENDER_LOOP.get_mut().unwrap().initialize(&mut context);
            let renderer = imgui_dx9::Renderer::new(&mut context, this.clone()).unwrap();

            #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
            let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongPtrA(
                renderer.get_hwnd(),
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as isize,
            ));

            #[cfg(target_arch = "x86")]
            let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongA(
                renderer.get_hwnd(),
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as i32,
            ));

            Mutex::new(Box::new(ImguiRenderer {
                ctx: context,
                renderer,
                wnd_proc,
                flags: ImguiRenderLoopFlags { focused: false },
            }))
        })
        .lock();

    imgui_renderer.render();
}

type Dx9EndSceneFn = unsafe extern "system" fn(this: IDirect3DDevice9) -> HRESULT;

type Dx9ResetFn =
    unsafe extern "system" fn(this: IDirect3DDevice9, *const D3DPRESENT_PARAMETERS) -> HRESULT;

type Dx9PresentFn = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

unsafe extern "system" fn imgui_dx9_reset_impl(
    this: IDirect3DDevice9,
    present_params: *const D3DPRESENT_PARAMETERS,
) -> HRESULT {
    trace!(
        "IDirect3DDevice9::Reset invoked ({} x {})",
        (*present_params).BackBufferWidth,
        (*present_params).BackBufferHeight
    );

    if let Some(renderer) = IMGUI_RENDERER.take() {
        renderer.lock().cleanup();
    }

    let (_, _, trampoline_reset) =
        TRAMPOLINE.get().expect("IDirect3DDevice9::Reset trampoline uninitialized");
    trampoline_reset(this, present_params)
}

unsafe extern "system" fn imgui_dx9_end_scene_impl(this: IDirect3DDevice9) -> HRESULT {
    trace!("IDirect3DDevice9::EndScene invoked");

    let mut viewport = core::mem::zeroed();
    this.GetViewport(&mut viewport).unwrap();
    let render_target_surface = this.GetRenderTarget(0).unwrap();
    let mut render_target_desc = core::mem::zeroed();
    render_target_surface.GetDesc(&mut render_target_desc).unwrap();

    let backbuffer_surface = this.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO).unwrap();
    let mut backbuffer_desc = core::mem::zeroed();
    backbuffer_surface.GetDesc(&mut backbuffer_desc).unwrap();

    trace!("Viewport: {:?}", viewport);
    trace!("Render target desc: {:?}", render_target_desc);
    trace!("Backbuffer desc: {:?}", backbuffer_desc);

    let (trampoline_end_scene, ..) =
        TRAMPOLINE.get().expect("IDirect3DDevice9::EndScene trampoline uninitialized");

    trampoline_end_scene(this)
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    if let Some(imgui_renderer) = IMGUI_RENDERER.get_mut().and_then(|m| m.try_lock()) {
        imgui_wnd_proc_impl(
            hwnd,
            umsg,
            WPARAM(wparam),
            LPARAM(lparam),
            imgui_renderer,
            IMGUI_RENDER_LOOP.get().unwrap(),
        )
    } else {
        debug!("WndProc called before hook was set");
        DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
    }
}

unsafe extern "system" fn imgui_dx9_present_impl(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    trace!("IDirect3DDevice9::Present invoked");

    this.BeginScene().unwrap();
    draw(&this);
    this.EndScene().unwrap();

    let (_, trampoline_present, _) =
        TRAMPOLINE.get().expect("IDirect3DDevice9::Present trampoline uninitialized");

    trampoline_present(this, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion)
}

static mut IMGUI_RENDER_LOOP: OnceLock<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceLock::new();
static mut IMGUI_RENDERER: OnceLock<Mutex<Box<ImguiRenderer>>> = OnceLock::new();
static TRAMPOLINE: OnceLock<(Dx9EndSceneFn, Dx9PresentFn, Dx9ResetFn)> = OnceLock::new();

struct ImguiRenderer {
    ctx: Context,
    renderer: imgui_dx9::Renderer,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
}

impl ImguiRenderer {
    unsafe fn render(&mut self) {
        if let Some(rect) = self.renderer.get_window_rect() {
            let io = self.ctx.io_mut();

            io.display_size = [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];

            let mut pos = POINT { x: 0, y: 0 };

            let active_window = GetForegroundWindow();
            if !HANDLE(active_window.0).is_invalid()
                && (active_window == self.renderer.get_hwnd()
                    || IsChild(active_window, self.renderer.get_hwnd()).as_bool())
            {
                let gcp = GetCursorPos(&mut pos as *mut _);
                if gcp.as_bool()
                    && ScreenToClient(self.renderer.get_hwnd(), &mut pos as *mut _).as_bool()
                {
                    io.mouse_pos[0] = pos.x as _;
                    io.mouse_pos[1] = pos.y as _;
                }
            }
        } else {
            trace!("GetWindowRect error: {:x}", GetLastError().0);
        }

        let ui = self.ctx.frame();

        IMGUI_RENDER_LOOP.get_mut().unwrap().render(ui, &self.flags);
        let draw_data = self.ctx.render();
        self.renderer.render(draw_data).unwrap();
    }

    unsafe fn cleanup(&mut self) {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        SetWindowLongPtrA(self.renderer.get_hwnd(), GWLP_WNDPROC, self.wnd_proc as usize as isize);

        #[cfg(target_arch = "x86")]
        SetWindowLongA(self.renderer.get_hwnd(), GWLP_WNDPROC, self.wnd_proc as usize as i32);
    }
}

impl ImguiWindowsEventHandler for ImguiRenderer {
    fn io(&self) -> &imgui::Io {
        self.ctx.io()
    }

    fn io_mut(&mut self) -> &mut imgui::Io {
        self.ctx.io_mut()
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

/// Stores hook detours and implements the [`Hooks`] trait.
pub struct ImguiDx9Hooks(MhHooks);

impl ImguiDx9Hooks {
    /// # Safety
    ///
    /// Is most likely undefined behavior, as it modifies function pointers at
    /// runtime.
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let (hook_dx9_end_scene_address, dx9_present_address, dx9_reset_address) =
            get_dx9_present_addr();

        let hook_dx9_end_scene =
            MhHook::new(hook_dx9_end_scene_address as *mut _, imgui_dx9_end_scene_impl as *mut _)
                .expect("couldn't create IDirect3DDevice9::EndScene hook");

        let hook_dx9_present =
            MhHook::new(dx9_present_address as *mut _, imgui_dx9_present_impl as *mut _)
                .expect("couldn't create IDirect3DDevice9::Present hook");

        let hook_dx9_reset =
            MhHook::new(dx9_reset_address as *mut _, imgui_dx9_reset_impl as *mut _)
                .expect("couldn't create IDirect3DDevice9::Reset hook");

        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| {
            (
                mem::transmute(hook_dx9_end_scene.trampoline()),
                mem::transmute(hook_dx9_present.trampoline()),
                mem::transmute(hook_dx9_reset.trampoline()),
            )
        });

        Self(
            MhHooks::new([hook_dx9_end_scene, hook_dx9_present, hook_dx9_reset])
                .expect("couldn't create hooks"),
        )
    }
}

impl Hooks for ImguiDx9Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        Box::new(unsafe { ImguiDx9Hooks::new(t) })
    }

    unsafe fn hook(&self) {
        self.0.apply();
    }

    unsafe fn unhook(&mut self) {
        self.0.unapply();

        if let Some(renderer) = IMGUI_RENDERER.take() {
            renderer.lock().cleanup();
        }
        drop(IMGUI_RENDER_LOOP.take());
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

unsafe fn get_dx9_present_addr() -> (Dx9EndSceneFn, Dx9PresentFn, Dx9ResetFn) {
    let d9 = Direct3DCreate9(D3D_SDK_VERSION).unwrap();

    let mut d3d_display_mode =
        D3DDISPLAYMODE { Width: 0, Height: 0, RefreshRate: 0, Format: D3DFORMAT(0) };
    d9.GetAdapterDisplayMode(D3DADAPTER_DEFAULT, &mut d3d_display_mode).unwrap();

    let mut present_params = D3DPRESENT_PARAMETERS {
        Windowed: BOOL(1),
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        BackBufferFormat: d3d_display_mode.Format,
        ..core::mem::zeroed()
    };

    let mut device: Option<IDirect3DDevice9> = None;
    d9.CreateDevice(
        D3DADAPTER_DEFAULT,
        D3DDEVTYPE_NULLREF,
        GetDesktopWindow(),
        D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
        &mut present_params,
        &mut device,
    )
    .expect("IDirect3DDevice9::CreateDevice: failed to create device");
    let device = device.unwrap();

    let end_scene_ptr = device.vtable().EndScene;
    let present_ptr = device.vtable().Present;
    let reset_ptr = device.vtable().Reset;

    (
        std::mem::transmute(end_scene_ptr),
        std::mem::transmute(present_ptr),
        std::mem::transmute(reset_ptr),
    )
}

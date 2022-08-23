use std::ffi::CString;
use std::ptr::null;
use std::sync::RwLock;

use detour::RawDetour;
use imgui::Context;
use log::{debug, error, trace};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use windows::core::{Interface, HRESULT, PCSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_HAL, D3DDISPLAYMODE, D3DFORMAT,
    D3DPRESENT_PARAMETERS, D3DSURFACE_DESC, D3DSWAPEFFECT_DISCARD, D3DVIEWPORT9, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::{ScreenToClient, HBRUSH, HDC, RGNDATA};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExA, DefWindowProcA, DefWindowProcW, DestroyWindow, GetCursorPos,
    GetForegroundWindow, IsChild, RegisterClassA, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, GWLP_WNDPROC,
    HCURSOR, HICON, HMENU, WINDOW_EX_STYLE, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::hooks::common::{imgui_wnd_proc_impl, ImguiWindowsEventHandler};
use crate::hooks::{Hooks, ImguiRenderLoop, ImguiRenderLoopFlags};

unsafe fn draw(this: &IDirect3DDevice9) {
    // Get the imgui renderer, or create it if it does not exist
    let mut imgui_renderer = IMGUI_RENDERER
        .get_or_insert_with(|| {
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
                game_hwnd: todo!(),
            }))
        })
        .lock();

    imgui_renderer.render();
}

type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

type OpenGl32wglSwapBuffers = unsafe extern "system" fn(HDC) -> ();

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    if IMGUI_RENDERER.is_some() {
        match IMGUI_RENDERER.as_mut().unwrap().try_lock() {
            Some(imgui_renderer) => imgui_wnd_proc_impl(
                hwnd,
                umsg,
                WPARAM(wparam),
                LPARAM(lparam),
                imgui_renderer,
                IMGUI_RENDER_LOOP.get().unwrap(),
            ),
            None => {
                debug!("Could not lock in WndProc");
                DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
            },
        }
    } else {
        debug!("WndProc called before hook was set");
        DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
    }
}

#[allow(non_snake_case)]
unsafe extern "system" fn imgui_opengl3_wglSwapBuffers_impl(dc: HDC) -> () {
    trace!("opengl32.wglSwapBuffers invoked");

    // Draw imgui
    // draw(&this);

    // Get the trampoline
    let trampoline_wglswapbuffers =
        TRAMPOLINE.get().expect("opengl32.wglSwapBuffers trampoline uninitialized");

    // Call the original function
    trampoline_wglswapbuffers(dc)
}

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static mut IMGUI_RENDERER: Option<Mutex<Box<ImguiRenderer>>> = None;
static mut GAME_HWND: Option<RwLock<Box<HWND>>> = None;
static TRAMPOLINE: OnceCell<OpenGl32wglSwapBuffers> = OnceCell::new();

struct ImguiRenderer {
    ctx: Context,
    renderer: imgui_dx9::Renderer,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
    game_hwnd: HWND,
}

impl ImguiRenderer {
    unsafe fn render(&mut self) {
        // USE GetWindowRect HERE ON THE HWND
        if let Some(rect) = self.renderer.get_window_rect() {
            let mut io = self.ctx.io_mut();

            io.display_size = [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];

            let mut pos = POINT { x: 0, y: 0 };

            let active_window = GetForegroundWindow();
            if !HANDLE(active_window.0).is_invalid()
                && (active_window == self.game_hwnd
                    || IsChild(active_window, self.game_hwnd).as_bool())
            {
                let gcp = GetCursorPos(&mut pos as *mut _);
                if gcp.as_bool() && ScreenToClient(self.game_hwnd, &mut pos as *mut _).as_bool() {
                    io.mouse_pos[0] = pos.x as _;
                    io.mouse_pos[1] = pos.y as _;
                }
            }
        } else {
            trace!("GetWindowRect error: {:x}", GetLastError().0);
        }

        let mut ui = self.ctx.frame();

        IMGUI_RENDER_LOOP.get_mut().unwrap().render(&mut ui, &self.flags);
        let draw_data = ui.render();
        self.renderer.render(draw_data).unwrap();
    }

    unsafe fn cleanup(&mut self) {}
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
pub struct OpenGL3Hooks {
    #[allow(dead_code)]
    hook_opengl_swapbuffers: RawDetour,
}

impl OpenGL3Hooks {
    /// # Safety
    ///
    /// Is most likely undefined behavior, as it modifies function pointers at
    /// runtime.
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        // Grab the addresses
        let hook_opengl_swapbuffers_address = get_opengl_wglswapbuffers_addr();

        // Create detours
        let hook_opengl_swapbuffers = RawDetour::new(
            hook_opengl_swapbuffers_address as *const _,
            imgui_opengl3_wglSwapBuffers_impl as *const _,
        )
        .expect("opengl32.wglSwapBuffers hook");

        // Initialize the render loop and store detours
        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| std::mem::transmute(hook_opengl_swapbuffers.trampoline()));

        Self { hook_opengl_swapbuffers }
    }
}

impl Hooks for OpenGL3Hooks {
    unsafe fn hook(&self) {
        for hook in [&self.hook_opengl_swapbuffers] {
            if let Err(e) = hook.enable() {
                error!("Couldn't enable hook: {e}");
            }
        }
    }

    unsafe fn unhook(&mut self) {
        for hook in [&self.hook_opengl_swapbuffers] {
            if let Err(e) = hook.disable() {
                error!("Couldn't disable hook: {e}");
            }
        }

        if let Some(renderer) = IMGUI_RENDERER.take() {
            renderer.lock().cleanup();
        }
        drop(IMGUI_RENDER_LOOP.take());
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////
unsafe fn get_opengl_wglswapbuffers_addr() -> OpenGl32wglSwapBuffers {
    // Grab a handle to opengl32.dll
    let opengl32dll = CString::new("opengl32.dll").unwrap();
    let opengl32module = GetModuleHandleA(PCSTR(opengl32dll.as_ptr() as *mut _))
        .expect("failed finding opengl32.dll");

    // Grab the address of wglSwapBuffers
    let wglswapbuffers = CString::new("wglSwapBuffers").unwrap();
    let wglswapbuffers_func =
        GetProcAddress(opengl32module, PCSTR(wglswapbuffers.as_ptr() as *mut _)).unwrap();

    std::mem::transmute(wglswapbuffers_func)
}

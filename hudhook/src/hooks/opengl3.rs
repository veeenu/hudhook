use std::ffi::CString;
use std::time::Instant;

use detour::RawDetour;
use imgui::Context;
use log::{debug, error, trace};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use windows::core::PCSTR;
use windows::Win32::Foundation::{
    GetLastError, BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ScreenToClient, WindowFromDC, HDC};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetForegroundWindow, GetWindowRect, IsChild, GWLP_WNDPROC,
};

use crate::hooks::common::{imgui_wnd_proc_impl, ImguiWindowsEventHandler};
use crate::hooks::{Hooks, ImguiRenderLoop, ImguiRenderLoopFlags};

unsafe fn draw(dc: HDC) {
    // Get the imgui renderer, or create it if it does not exist
    let mut imgui_renderer = IMGUI_RENDERER
        .get_or_insert_with(|| {
            // Create ImGui context
            let mut context = imgui::Context::create();
            context.set_ini_filename(None);

            // Initialize the render loop with the context
            IMGUI_RENDER_LOOP.get_mut().unwrap().initialize(&mut context);

            // Init the OpenGL loader (used for grabbing the OpenGL functions)
            gl_loader::init_gl();
            let renderer = imgui_opengl_renderer::Renderer::new(&mut context, |s| {
                gl_loader::get_proc_address(s) as _
            });

            // Grab the HWND from the DC
            let hwnd = WindowFromDC(dc);

            // Set the new wnd proc, and assign the old one to a variable for further
            // storing
            #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
            let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongPtrA(
                hwnd,
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as isize,
            ));
            #[cfg(target_arch = "x86")]
            let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongA(
                hwnd,
                GWLP_WNDPROC,
                imgui_wnd_proc as usize as i32,
            ));

            // Create the imgui rendererer
            let mut imgui_renderer = ImguiRenderer {
                ctx: context,
                renderer,
                wnd_proc,
                flags: ImguiRenderLoopFlags { focused: false },
                game_hwnd: hwnd,
            };

            // Initialize window events on the imgui renderer
            ImguiWindowsEventHandler::setup_io(&mut imgui_renderer);

            // Return the imgui renderer as a mutex
            Mutex::new(Box::new(imgui_renderer))
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
unsafe extern "system" fn imgui_opengl32_wglSwapBuffers_impl(dc: HDC) -> () {
    trace!("opengl32.wglSwapBuffers invoked");

    // Draw ImGui
    draw(dc);

    // Get the trampoline
    let trampoline_wglswapbuffers =
        TRAMPOLINE.get().expect("opengl32.wglSwapBuffers trampoline uninitialized");

    // Call the original function
    trampoline_wglswapbuffers(dc)
}

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static mut IMGUI_RENDERER: Option<Mutex<Box<ImguiRenderer>>> = None;
static TRAMPOLINE: OnceCell<OpenGl32wglSwapBuffers> = OnceCell::new();

struct ImguiRenderer {
    ctx: Context,
    renderer: imgui_opengl_renderer::Renderer,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
    game_hwnd: HWND,
}

fn get_window_rect(hwnd: &HWND) -> Option<RECT> {
    unsafe {
        let mut rect: RECT = RECT { ..core::mem::zeroed() };
        if GetWindowRect(*hwnd, &mut rect) != BOOL(0) {
            Some(rect)
        } else {
            None
        }
    }
}

static mut LAST_FRAME: Option<Mutex<Instant>> = None;

impl ImguiRenderer {
    unsafe fn render(&mut self) {
        if let Some(rect) = get_window_rect(&self.game_hwnd) {
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

        // Update the delta time of ImGui as to tell it how long has elapsed since the
        // last frame
        let last_frame = LAST_FRAME.get_or_insert_with(|| Mutex::new(Instant::now())).get_mut();
        let now = Instant::now();
        self.ctx.io_mut().update_delta_time(now.duration_since(*last_frame));
        *last_frame = now;

        let mut ui = self.ctx.frame();

        IMGUI_RENDER_LOOP.get_mut().unwrap().render(&mut ui, &self.flags);
        self.renderer.render(ui);
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

// Get the address of wglSwapBuffers in opengl32.dll
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

/// Stores hook detours and implements the [`Hooks`] trait.
pub struct ImguiOpenGl3Hooks {
    hook_opengl_wgl_swap_buffers: RawDetour,
}

impl ImguiOpenGl3Hooks {
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
        let hook_opengl_wgl_swap_buffers = RawDetour::new(
            hook_opengl_swapbuffers_address as *const _,
            imgui_opengl32_wglSwapBuffers_impl as *const _,
        )
        .expect("opengl32.wglSwapBuffers hook");

        // Initialize the render loop and store detours
        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| std::mem::transmute(hook_opengl_wgl_swap_buffers.trampoline()));

        Self { hook_opengl_wgl_swap_buffers }
    }
}

impl Hooks for ImguiOpenGl3Hooks {
    unsafe fn hook(&self) {
        for hook in [&self.hook_opengl_wgl_swap_buffers] {
            if let Err(e) = hook.enable() {
                error!("Couldn't enable hook: {e}");
            }
        }
    }

    unsafe fn unhook(&mut self) {
        for hook in [&self.hook_opengl_wgl_swap_buffers] {
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

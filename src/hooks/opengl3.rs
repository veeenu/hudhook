use std::ffi::CString;
use std::time::Instant;

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{debug, trace};
use windows::core::PCSTR;
use windows::Win32::Foundation::{GetLastError, BOOL, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{WindowFromDC, HDC};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use super::common::{self, CURSOR_POS, KEYS, LAST_CURSOR_POS};
use crate::hooks::common::ImguiWindowsEventHandler;
use crate::hooks::{Hooks, ImguiRenderLoop, ImguiRenderLoopFlags};
use crate::mh::{MhHook, MhHooks};
use crate::renderers::imgui_opengl3::get_proc_address;

unsafe fn draw(dc: HDC) {
    // Get the imgui renderer, or create it if it does not exist
    let mut imgui_renderer = IMGUI_RENDERER
        .get_or_insert_with(|| {
            // Create ImGui context
            let mut context = imgui::Context::create();
            context.set_ini_filename(None);

            // Initialize the render loop with the context
            IMGUI_RENDER_LOOP.get_mut().unwrap().initialize(&mut context);

            let renderer = imgui_opengl::Renderer::new(&mut context, |s| {
                get_proc_address(CString::new(s).unwrap()) as _
            });

            // Grab the HWND from the DC
            let hwnd = WindowFromDC(dc);

            // Create the imgui rendererer
            let mut imgui_renderer = ImguiRenderer {
                ctx: context,
                renderer,

                flags: ImguiRenderLoopFlags { focused: false },
                game_hwnd: hwnd,
            };

            LAST_CURSOR_POS.get_or_init(|| Mutex::new(POINT { x: 0, y: 0 }));
            CURSOR_POS.get_or_init(|| Mutex::new(POINT { x: 0, y: 0 }));
            KEYS.get_or_init(|| Mutex::new([0x08; 256]));

            // Initialize window events on the imgui renderer
            ImguiWindowsEventHandler::setup_io(&mut imgui_renderer);

            common::hook_msg_proc();

            // Return the imgui renderer as a mutex
            Mutex::new(Box::new(imgui_renderer))
        })
        .lock();

    imgui_renderer.render();
}

type OpenGl32wglSwapBuffers = unsafe extern "system" fn(HDC) -> ();

#[allow(non_snake_case)]
unsafe extern "system" fn imgui_opengl32_wglSwapBuffers_impl(dc: HDC) {
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
    renderer: imgui_opengl::Renderer,

    flags: ImguiRenderLoopFlags,
    game_hwnd: HWND,
}

fn get_client_rect(hwnd: &HWND) -> Option<RECT> {
    unsafe {
        let mut rect: RECT = RECT { ..core::mem::zeroed() };
        if GetClientRect(*hwnd, &mut rect) != BOOL(0) {
            Some(rect)
        } else {
            None
        }
    }
}

static mut LAST_FRAME: Option<Mutex<Instant>> = None;

impl ImguiRenderer {
    unsafe fn render(&mut self) {
        let render_loop = IMGUI_RENDER_LOOP.get_mut().unwrap();

        if let Some(rect) = get_client_rect(&self.game_hwnd) {
            ImguiWindowsEventHandler::update_io(self, render_loop, self.game_hwnd, rect);
        } else {
            trace!("GetWindowRect error: {:x}", GetLastError().0);
        }

        // Update the delta time of ImGui as to tell it how long has elapsed since the
        // last frame
        let last_frame = LAST_FRAME.get_or_insert_with(|| Mutex::new(Instant::now())).get_mut();
        let now = Instant::now();
        self.ctx.io_mut().update_delta_time(now.duration_since(*last_frame));
        *last_frame = now;

        let ui = self.ctx.frame();

        render_loop.render(ui, &self.flags);
        self.renderer.render(&mut self.ctx);
    }

    unsafe fn cleanup(&mut self) {
        common::unhook_msg_proc();
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
pub struct ImguiOpenGl3Hooks(MhHooks);

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
        let hook_opengl_wgl_swap_buffers = MhHook::new(
            hook_opengl_swapbuffers_address as *mut _,
            imgui_opengl32_wglSwapBuffers_impl as *mut _,
        )
        .expect("couldn't create opengl32.wglSwapBuffers hook");

        // Initialize the render loop and store detours
        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| std::mem::transmute(hook_opengl_wgl_swap_buffers.trampoline()));

        Self(MhHooks::new([hook_opengl_wgl_swap_buffers]).expect("couldn't create hooks"))
    }
}

impl Hooks for ImguiOpenGl3Hooks {
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

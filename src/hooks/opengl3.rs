use std::ffi::CString;
use std::mem;
use std::sync::OnceLock;

use tracing::trace;
use windows::core::PCSTR;
use windows::Win32::Graphics::Gdi::{WindowFromDC, HDC};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

use crate::mh::MhHook;
use crate::renderer::RenderState;
use crate::{Hooks, ImguiRenderLoop};

type OpenGl32wglSwapBuffersType = unsafe extern "system" fn(HDC) -> ();

struct Trampolines {
    opengl32_wgl_swap_buffers: OpenGl32wglSwapBuffersType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();

unsafe extern "system" fn opengl32_wgl_swap_buffers_impl(dc: HDC) {
    let Trampolines { opengl32_wgl_swap_buffers } =
        TRAMPOLINES.get().expect("OpenGL3 trampolines uninitialized");

    // Don't attempt a render if one is already underway: it might be that the
    // renderer itself is currently invoking `Present`.
    if RenderState::is_locked() {
        return opengl32_wgl_swap_buffers(dc);
    }

    let hwnd = RenderState::setup(|| WindowFromDC(dc));

    RenderState::render(hwnd);

    trace!("Call OpenGL3 wglSwapBuffers trampoline");
    opengl32_wgl_swap_buffers(dc);
}

// Get the address of wglSwapBuffers in opengl32.dll
unsafe fn get_opengl_wglswapbuffers_addr() -> OpenGl32wglSwapBuffersType {
    // Grab a handle to opengl32.dll
    let opengl32dll = CString::new("opengl32.dll").unwrap();
    let opengl32module = GetModuleHandleA(PCSTR(opengl32dll.as_ptr() as *mut _))
        .expect("failed finding opengl32.dll");

    // Grab the address of wglSwapBuffers
    let wglswapbuffers = CString::new("wglSwapBuffers").unwrap();
    let wglswapbuffers_func =
        GetProcAddress(opengl32module, PCSTR(wglswapbuffers.as_ptr() as *mut _)).unwrap();

    mem::transmute(wglswapbuffers_func)
}

/// Stores hook detours and implements the [`Hooks`] trait.
pub struct ImguiOpenGl3Hooks([MhHook; 1]);

impl ImguiOpenGl3Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `opengl32::wglSwapBuffers`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        // Grab the addresses
        let hook_opengl_swap_buffers_address = get_opengl_wglswapbuffers_addr();

        // Create detours
        let hook_opengl_wgl_swap_buffers = MhHook::new(
            hook_opengl_swap_buffers_address as *mut _,
            opengl32_wgl_swap_buffers_impl as *mut _,
        )
        .expect("couldn't create opengl32.wglSwapBuffers hook");

        // Initialize the render loop and store detours
        RenderState::set_render_loop(t);
        TRAMPOLINES.get_or_init(|| Trampolines {
            opengl32_wgl_swap_buffers: std::mem::transmute(
                hook_opengl_wgl_swap_buffers.trampoline(),
            ),
        });

        Self([hook_opengl_wgl_swap_buffers])
    }
}

impl Hooks for ImguiOpenGl3Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        Box::new(unsafe { ImguiOpenGl3Hooks::new(t) })
    }

    fn hooks(&self) -> &[MhHook] {
        &self.0
    }

    unsafe fn unhook(&mut self) {
        RenderState::cleanup();
        TRAMPOLINES.take();
    }
}

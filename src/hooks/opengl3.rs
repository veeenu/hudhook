use std::ffi::CString;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Result, HRESULT, PCSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{WindowFromDC, HDC};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW};

use crate::compositor::opengl3::Compositor;
use crate::mh::MhHook;
use crate::pipeline::{Pipeline, PipelineMessage, PipelineSharedState};
use crate::{util, Hooks, ImguiRenderLoop};

type OpenGl32wglSwapBuffersType = unsafe extern "system" fn(HDC) -> ();

struct Trampolines {
    opengl32_wgl_swap_buffers: OpenGl32wglSwapBuffersType,
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> =
    OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

unsafe fn init_pipeline(
    dc: HDC,
) -> Result<(Mutex<Pipeline<Compositor>>, Arc<PipelineSharedState>)> {
    let hwnd = WindowFromDC(dc);
    let compositor = Compositor::new()?;

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

fn render(dc: HDC) -> Result<()> {
    let (pipeline, _) = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(dc)) }?;

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

unsafe extern "system" fn opengl32_wgl_swap_buffers_impl(dc: HDC) {
    let Trampolines { opengl32_wgl_swap_buffers } =
        TRAMPOLINES.get().expect("OpenGL3 trampolines uninitialized");

    if let Err(e) = render(dc) {
        util::print_dxgi_debug_messages();
        error!("Render error: {e:?}");
    }

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
        RENDER_LOOP.get_or_init(move || Box::new(t));
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
        TRAMPOLINES.take();
        PIPELINE.take();
        RENDER_LOOP.take();
    }
}

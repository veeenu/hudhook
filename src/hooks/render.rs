use std::{
    mem,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use parking_lot::Mutex;
use tracing::{debug, error};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{DefWindowProcW, GWLP_WNDPROC},
};

use crate::hooks::input::{imgui_wnd_proc_impl, WndProcType};
use crate::{renderer::dx12::RenderEngine, ImguiRenderLoop};

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut WND_PROC: OnceLock<WndProcType> = OnceLock::new();
static mut RENDER_ENGINE: OnceLock<Mutex<RenderEngine>> = OnceLock::new();
static mut RENDER_LOOP: OnceLock<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceLock::new();
static RENDER_LOCK: AtomicBool = AtomicBool::new(false);

pub(super) struct RenderState;

impl RenderState {
    pub(super) fn setup<F: Fn() -> HWND>(f: F) -> HWND {
        let hwnd = unsafe { *GAME_HWND.get_or_init(f) };

        unsafe {
            WND_PROC.get_or_init(|| {
                #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
                let wnd_proc =
                    mem::transmute(windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA(
                        hwnd,
                        GWLP_WNDPROC,
                        imgui_wnd_proc as usize as isize,
                    ));

                #[cfg(target_arch = "x86")]
                let wnd_proc =
                    mem::transmute(windows::Win32::UI::WindowsAndMessaging::SetWindowLongA(
                        hwnd,
                        GWLP_WNDPROC,
                        imgui_wnd_proc as usize as i32,
                    ));

                wnd_proc
            })
        };

        hwnd
    }

    pub(super) fn set_render_loop<T: ImguiRenderLoop + Send + Sync + 'static>(t: T) {
        unsafe { RENDER_LOOP.get_or_init(|| Box::new(t)) };
    }

    pub(super) fn is_locked() -> bool {
        RENDER_LOCK.load(Ordering::SeqCst)
    }

    pub(super) fn render(hwnd: HWND) {
        RENDER_LOCK.store(true, Ordering::SeqCst);

        let render_engine = unsafe {
            RENDER_ENGINE.get_or_init(move || Mutex::new(RenderEngine::new(hwnd).unwrap()))
        };

        let Some(mut render_engine) = render_engine.try_lock() else {
            error!("Could not lock render engine");
            return;
        };
        let Some(render_loop) = (unsafe { RENDER_LOOP.get_mut() }) else {
            error!("Could not obtain render loop");
            return;
        };

        if let Err(e) = render_engine.render(|ui| render_loop.render(ui)) {
            error!("Render: {e:?}");
        }

        RENDER_LOCK.store(false, Ordering::SeqCst);
    }

    pub(super) fn cleanup() {
        unsafe {
            RENDER_ENGINE.take();
            RENDER_LOOP.take();
            RENDER_LOCK.store(false, Ordering::SeqCst);
        }
    }
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    let render_engine = match RENDER_ENGINE.get().map(Mutex::try_lock) {
        Some(Some(render_engine)) => render_engine,
        Some(None) => {
            debug!("Could not lock in WndProc");
            return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
        },
        None => {
            debug!("WndProc called before hook was set");
            return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
        },
    };

    let Some(render_loop) = RENDER_LOOP.get() else {
        debug!("Could not get render loop");
        return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
    };

    let Some(&wnd_proc) = WND_PROC.get() else {
        debug!("Could not get original WndProc");
        return DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
    };

    imgui_wnd_proc_impl(
        hwnd,
        umsg,
        WPARAM(wparam),
        LPARAM(lparam),
        wnd_proc,
        render_engine,
        render_loop,
    )
}

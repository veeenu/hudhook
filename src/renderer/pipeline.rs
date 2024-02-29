use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use imgui::Context;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Result, HRESULT};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GetCursorPos, GetForegroundWindow, IsChild, SetWindowLongPtrA,
    GWLP_WNDPROC,
};

use crate::renderer::input::{imgui_wnd_proc_impl, WndProcType};
use crate::renderer::{keys, RenderEngine};
use crate::{util, ImguiRenderLoop};

type RenderLoop = Box<dyn ImguiRenderLoop + Send + Sync>;

static mut PIPELINE_STATES: Lazy<Mutex<HashMap<isize, Arc<PipelineSharedState>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct PipelineMessage(pub HWND, pub u32, pub WPARAM, pub LPARAM);

pub struct PipelineSharedState {
    pub should_block_events: AtomicBool,
    pub wnd_proc: WndProcType,
    pub tx: Sender<PipelineMessage>,
}

pub struct Pipeline<T: RenderEngine> {
    hwnd: HWND,
    ctx: Context,
    engine: T,
    render_loop: RenderLoop,
    rx: Receiver<PipelineMessage>,
    shared_state: Arc<PipelineSharedState>,
}

impl<T: RenderEngine> Pipeline<T> {
    pub fn new(
        hwnd: HWND,
        mut ctx: Context,
        mut engine: T,
        mut render_loop: RenderLoop,
    ) -> std::result::Result<Self, (Error, RenderLoop)> {
        let (width, height) = util::win_size(hwnd);

        ctx.io_mut().display_size = [width as f32, height as f32];

        render_loop.initialize(&mut ctx, &mut |data, width, height| {
            engine.load_image(data, width, height)
        });

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let wnd_proc = unsafe {
            mem::transmute(SetWindowLongPtrA(
                hwnd,
                GWLP_WNDPROC,
                pipeline_wnd_proc as usize as isize,
            ))
        };

        // TODO is this necessary? SetWindowLongPtrA should already decay to
        // SetWindowLongA
        #[cfg(target_arch = "x86")]
        let wnd_proc = unsafe {
            mem::transmute(SetWindowLongA(hwnd, GWLP_WNDPROC, pipeline_wnd_proc as usize as i32))
        };

        let (tx, rx) = mpsc::channel();
        let shared_state = Arc::new(PipelineSharedState {
            should_block_events: AtomicBool::new(false),
            wnd_proc,
            tx,
        });

        unsafe { PIPELINE_STATES.lock() }.insert(hwnd.0, Arc::clone(&shared_state));

        Ok(Self { hwnd, ctx, engine, render_loop, rx, shared_state: Arc::clone(&shared_state) })
    }

    pub fn prepare_render(&mut self) -> Result<()> {
        // TODO find a better alternative than allocating each frame
        let message_queue = self.rx.try_iter().collect::<Vec<_>>();

        message_queue.into_iter().for_each(|PipelineMessage(hwnd, umsg, wparam, lparam)| {
            imgui_wnd_proc_impl(hwnd, umsg, wparam, lparam, self);
        });

        let should_block_events = self.render_loop.should_block_messages(self.ctx.io_mut());

        self.shared_state.should_block_events.store(should_block_events, Ordering::SeqCst);

        let io = self.ctx.io_mut();

        unsafe {
            let active_window = GetForegroundWindow();
            if active_window == self.hwnd
                || (!HANDLE(active_window.0).is_invalid()
                    && IsChild(active_window, self.hwnd).as_bool())
            {
                let mut pos = util::try_out_param(|v| GetCursorPos(v))?;
                if ScreenToClient(self.hwnd, &mut pos).as_bool() {
                    io.mouse_pos = [pos.x as f32, pos.y as f32];
                }
            }
        }

        io.nav_active = true;
        io.nav_visible = true;

        for (key, virtual_key) in keys::KEYS {
            io[key] = virtual_key.0 as u32;
        }

        self.render_loop.before_render(&mut self.ctx);

        Ok(())
    }

    pub fn render(&mut self, render_target: T::RenderTarget) -> Result<()> {
        let [w, h] = self.ctx.io().display_size;
        let [fsw, fsh] = self.ctx.io().display_framebuffer_scale;

        if (w * fsw) <= 0.0 || (h * fsh) <= 0.0 {
            error!("Insufficient display size: {w}x{h}");
            return Err(Error::new(HRESULT(-1), "Insufficient display size".into()));
        }

        let ui = self.ctx.frame();
        self.render_loop.render(ui);
        let draw_data = self.ctx.render();

        self.engine.render(draw_data, render_target)?;

        Ok(())
    }

    pub fn engine(&self) -> &T {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut T {
        &mut self.engine
    }

    pub fn context(&mut self) -> &mut Context {
        &mut self.ctx
    }

    pub fn render_loop(&mut self) -> &mut RenderLoop {
        &mut self.render_loop
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        trace!("RESIZING TO {width} x {height}");
        // let (width, height) = util::win_size(self.hwnd);
        // let (width, height) = (width as u32, height as u32);

        let io = self.ctx.io_mut();

        // if let Err(e) = self.engine.resize(width, height) {
        //     error!("Couldn't resize engine: {e:?}");
        // }

        io.display_size = [width as f32, height as f32];
    }

    pub fn cleanup(&mut self) {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        unsafe {
            SetWindowLongPtrA(self.hwnd, GWLP_WNDPROC, self.shared_state.wnd_proc as usize as isize)
        };

        // TODO is this necessary? SetWindowLongPtrA should already decay to
        // SetWindowLongA
        #[cfg(target_arch = "x86")]
        unsafe {
            SetWindowLongA(self.hwnd, GWLP_WNDPROC, self.shared_state.wnd_proc as usize as i32)
        };
    }
}

impl<T: RenderEngine> Drop for Pipeline<T> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

unsafe extern "system" fn pipeline_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let shared_state = {
        let Some(shared_state_guard) = PIPELINE_STATES.try_lock() else {
            error!("No lockerino?");
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };

        let Some(shared_state) = shared_state_guard.get(&hwnd.0) else {
            error!("No shared state?");
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };

        Arc::clone(shared_state)
    };

    if let Err(e) = shared_state.tx.send(PipelineMessage(hwnd, msg, wparam, lparam)) {
        error!("No senderino? {e:?}");
    }

    // CONCURRENCY: as the message interpretation now happens out of band, this
    // expresses the intent as of *before* the current message was received.
    let should_block_messages = shared_state.should_block_events.load(Ordering::SeqCst);

    if should_block_messages {
        LRESULT(1)
    } else {
        CallWindowProcW(Some(shared_state.wnd_proc), hwnd, msg, wparam, lparam)
    }
}
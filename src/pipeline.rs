use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use imgui::Context;
use tracing::error;
use windows::core::{Error, Result, HRESULT};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrA, GWLP_WNDPROC};

use crate::renderer::input::{imgui_wnd_proc_impl, WndProcType};
use crate::renderer::RenderEngine;
use crate::{util, ImguiRenderLoop};

type RenderLoop = Box<dyn ImguiRenderLoop + Send + Sync>;

pub struct PipelineMessage(pub HWND, pub u32, pub WPARAM, pub LPARAM);

pub struct PipelineSharedState {
    pub should_block_events: AtomicBool,
    pub wnd_proc: WndProcType,
    pub tx: Sender<PipelineMessage>,
}

pub struct Pipeline<T> {
    hwnd: HWND,
    compositor: T,
    ctx: Context,
    engine: RenderEngine,
    render_loop: RenderLoop,
    rx: Receiver<PipelineMessage>,
    shared_state: Arc<PipelineSharedState>,
}

impl<T> Pipeline<T> {
    pub fn new(
        hwnd: HWND,
        wnd_proc: WndProcType,
        compositor: T,
        mut render_loop: RenderLoop,
    ) -> std::result::Result<(Self, Arc<PipelineSharedState>), (Error, RenderLoop)> {
        let (width, height) = util::win_size(hwnd);

        let mut ctx = Context::create();
        ctx.io_mut().display_size = [width as f32, height as f32];

        let mut engine = match RenderEngine::new(&mut ctx, width as u32, height as u32) {
            Ok(engine) => engine,
            Err(e) => return Err((e, render_loop)),
        };

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let wnd_proc = unsafe {
            mem::transmute(SetWindowLongPtrA(hwnd, GWLP_WNDPROC, wnd_proc as usize as isize))
        };

        // TODO is this necessary? SetWindowLongPtrA should already decay to
        // SetWindowLongA
        #[cfg(target_arch = "x86")]
        let wnd_proc =
            unsafe { mem::transmute(SetWindowLongA(hwnd, GWLP_WNDPROC, wnd_proc as usize as i32)) };

        render_loop.initialize(&mut engine);

        let (tx, rx) = mpsc::channel();
        let shared_state = Arc::new(PipelineSharedState {
            should_block_events: AtomicBool::new(false),
            wnd_proc,
            tx,
        });

        Ok((
            Self {
                hwnd,
                compositor,
                ctx,
                engine,
                render_loop,
                rx,
                shared_state: Arc::clone(&shared_state),
            },
            shared_state,
        ))
    }

    pub fn render(&mut self) -> Result<ID3D12Resource> {
        // TODO find a better alternative than allocating each frame
        let message_queue = self.rx.try_iter().collect::<Vec<_>>();

        message_queue.into_iter().for_each(|PipelineMessage(hwnd, umsg, wparam, lparam)| {
            imgui_wnd_proc_impl(hwnd, umsg, wparam, lparam, self);
        });

        let should_block_events = self.render_loop.should_block_messages(self.ctx.io_mut());

        self.shared_state.should_block_events.store(should_block_events, Ordering::SeqCst);

        self.engine.render_setup(self.hwnd, &mut self.ctx)?;
        self.render_loop.before_render(&mut self.engine);

        let [w, h] = self.ctx.io().display_size;
        if w <= 0.0 || h <= 0.0 {
            error!("Insufficient display size: {w}x{h}");
            return Err(Error::new(HRESULT(-1), "Insufficient display size".into()));
        }

        let ui = self.ctx.frame();
        self.render_loop.render(ui);
        let draw_data = self.ctx.render();

        self.engine.render(draw_data)
    }

    pub fn engine(&self) -> &RenderEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut RenderEngine {
        &mut self.engine
    }

    pub fn context(&mut self) -> &mut Context {
        &mut self.ctx
    }

    pub fn compositor(&self) -> &T {
        &self.compositor
    }

    pub fn compositor_mut(&mut self) -> &mut T {
        &mut self.compositor
    }

    pub fn render_loop(&mut self) -> &mut RenderLoop {
        &mut self.render_loop
    }

    pub fn resize(&mut self) {
        let (width, height) = util::win_size(self.hwnd);
        let (width, height) = (width as u32, height as u32);

        let io = self.ctx.io_mut();

        if let Err(e) = self.engine.resize(width, height) {
            error!("Couldn't resize engine: {e:?}");
        }

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

impl<T> Drop for Pipeline<T> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

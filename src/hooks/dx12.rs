//! Hook for DirectX 12 applications.
use std::ffi::c_void;
use std::mem::{self, ManuallyDrop};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use imgui::Context;
use log::*;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use widestring::{u16cstr, U16CStr};
use windows::core::{Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory, IDXGIFactory, DXGI_SWAP_CHAIN_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT, *,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::{CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT};
use windows::Win32::System::WindowsProgramming::INFINITE;
#[cfg(target_arch = "x86")]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongA;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrA;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::hooks::common::{
    imgui_wnd_proc_impl, Fence, ImguiRenderLoop, ImguiRenderLoopFlags, ImguiWindowsEventHandler,
    WndProcType,
};
use crate::hooks::Hooks;
use crate::mh::{MhHook, MhHooks};
use crate::renderers::imgui_dx12::RenderEngine;

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain3, SyncInterval: u32, Flags: u32) -> HRESULT;

type ExecuteCommandListsType = unsafe extern "system" fn(
    This: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
);

type ResizeBuffersType = unsafe extern "system" fn(
    This: IDXGISwapChain3,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Data structures and traits
////////////////////////////////////////////////////////////////////////////////////////////////////

trait Renderer {
    /// Invoked once per frame.
    fn render(&mut self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global singletons
////////////////////////////////////////////////////////////////////////////////////////////////////

static TRAMPOLINE: OnceCell<(
    DXGISwapChainPresentType,
    ExecuteCommandListsType,
    ResizeBuffersType,
)> = OnceCell::new();

const COMMAND_ALLOCATOR_NAMES: [&U16CStr; 8] = [
    u16cstr!("hudhook Command allocator #0"),
    u16cstr!("hudhook Command allocator #1"),
    u16cstr!("hudhook Command allocator #2"),
    u16cstr!("hudhook Command allocator #3"),
    u16cstr!("hudhook Command allocator #4"),
    u16cstr!("hudhook Command allocator #5"),
    u16cstr!("hudhook Command allocator #6"),
    u16cstr!("hudhook Command allocator #7"),
];

////////////////////////////////////////////////////////////////////////////////////////////////////
// Debugging
////////////////////////////////////////////////////////////////////////////////////////////////////

unsafe fn print_dxgi_debug_messages() {
    let diq: IDXGIInfoQueue = DXGIGetDebugInterface1(0).unwrap();

    for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
        let mut msg_len: usize = 0;
        diq.GetMessage(DXGI_DEBUG_ALL, i, null_mut(), &mut msg_len as _).unwrap();
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        diq.GetMessage(DXGI_DEBUG_ALL, i, pdiqm, &mut msg_len as _).unwrap();
        let diqm = pdiqm.as_ref().unwrap();
        debug!(
            "[DIQ] {}",
            String::from_utf8_lossy(std::slice::from_raw_parts(
                diqm.pDescription as *const u8,
                diqm.DescriptionByteLength - 1
            ))
        );
    }
    diq.ClearStoredMessages(DXGI_DEBUG_ALL);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hook entry points
////////////////////////////////////////////////////////////////////////////////////////////////////

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static mut IMGUI_RENDERER: OnceCell<Mutex<Box<ImguiRenderer>>> = OnceCell::new();
static mut COMMAND_QUEUE_GUARD: OnceCell<()> = OnceCell::new();
static DXGI_DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

static CQECL_RUNNING: Fence = Fence::new();
static PRESENT_RUNNING: Fence = Fence::new();
static RBUF_RUNNING: Fence = Fence::new();

#[derive(Debug)]
struct FrameContext {
    back_buffer: ID3D12Resource,
    desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_val: u64,
    fence_event: HANDLE,
}

impl FrameContext {
    fn incr(&mut self) {
        static FENCE_MAX: AtomicU64 = AtomicU64::new(0);
        self.fence_val = FENCE_MAX.fetch_add(1, Ordering::SeqCst);
    }

    fn wait_fence(&mut self) {
        unsafe {
            if self.fence.GetCompletedValue() < self.fence_val {
                self.fence.SetEventOnCompletion(self.fence_val, self.fence_event).unwrap();
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
        }
    }
}

unsafe extern "system" fn imgui_execute_command_lists_impl(
    cmd_queue: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
) {
    let _fence = CQECL_RUNNING.lock();

    trace!(
        "ID3D12CommandQueue::ExecuteCommandLists({cmd_queue:?}, {num_command_lists}, \
         {command_lists:p}) invoked"
    );
    COMMAND_QUEUE_GUARD
        .get_or_try_init(|| {
            let desc = cmd_queue.GetDesc();
            trace!("CommandQueue description: {:?}", desc);

            if desc.Type.0 != 0 {
                trace!("Skipping CommandQueue");
                return Err(());
            }

            if let Some(renderer) = IMGUI_RENDERER.get() {
                trace!("cmd_queue ptr was set");
                renderer.lock().command_queue = Some(cmd_queue.clone());
                Ok(())
            } else {
                trace!("cmd_queue ptr was not set: renderer not initialized");
                Err(())
            }
        })
        .ok();

    let (_, trampoline, _) =
        TRAMPOLINE.get().expect("ID3D12CommandQueue::ExecuteCommandLists trampoline uninitialized");
    trampoline(cmd_queue, num_command_lists, command_lists);
}

unsafe extern "system" fn imgui_dxgi_swap_chain_present_impl(
    swap_chain: IDXGISwapChain3,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let _fence = PRESENT_RUNNING.lock();

    let (trampoline_present, ..) =
        TRAMPOLINE.get().expect("IDXGISwapChain::Present trampoline uninitialized");

    trace!("IDXGISwapChain3::Present({swap_chain:?}, {sync_interval}, {flags}) invoked");

    let renderer =
        IMGUI_RENDERER.get_or_init(|| Mutex::new(Box::new(ImguiRenderer::new(swap_chain.clone()))));

    {
        renderer.lock().render(Some(swap_chain.clone()));
    }

    trace!("Invoking IDXGISwapChain3::Present trampoline");
    let r = trampoline_present(swap_chain, sync_interval, flags);
    trace!("Trampoline returned {:?}", r);

    // Windows + R -> dxcpl.exe
    // Edit list... -> add eldenring.exe
    // DXGI debug layer -> Force On
    if DXGI_DEBUG_ENABLED.load(Ordering::SeqCst) {
        print_dxgi_debug_messages();
    }

    r
}

unsafe extern "system" fn imgui_resize_buffers_impl(
    swap_chain: IDXGISwapChain3,
    buffer_count: u32,
    width: u32,
    height: u32,
    new_format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    let _fence = RBUF_RUNNING.lock();

    trace!("IDXGISwapChain3::ResizeBuffers invoked");
    let (_, _, trampoline) =
        TRAMPOLINE.get().expect("IDXGISwapChain3::ResizeBuffer trampoline uninitialized");

    if let Some(mutex) = IMGUI_RENDERER.take() {
        mutex.lock().cleanup(Some(swap_chain.clone()));
    };

    COMMAND_QUEUE_GUARD.take();

    trampoline(swap_chain, buffer_count, width, height, new_format, flags)
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    trace!("Entering WndProc {:x} {:x} {:x} {:x}", hwnd.0, umsg, wparam, lparam);

    match IMGUI_RENDERER.get().map(Mutex::try_lock) {
        Some(Some(imgui_renderer)) => imgui_wnd_proc_impl(
            hwnd,
            umsg,
            WPARAM(wparam),
            LPARAM(lparam),
            imgui_renderer,
            IMGUI_RENDER_LOOP.get().unwrap(),
        ),
        Some(None) => {
            debug!("Could not lock in WndProc");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        },
        None => {
            debug!("WndProc called before hook was set");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        },
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Render loops
////////////////////////////////////////////////////////////////////////////////////////////////////

struct ImguiRenderer {
    ctx: Context,
    engine: RenderEngine,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
    frame_contexts: Vec<FrameContext>,
    _rtv_heap: ID3D12DescriptorHeap,
    renderer_heap: ID3D12DescriptorHeap,
    command_queue: Option<ID3D12CommandQueue>,
    command_list: ID3D12GraphicsCommandList,
    swap_chain: IDXGISwapChain3,
}

impl ImguiRenderer {
    unsafe fn new(swap_chain: IDXGISwapChain3) -> Self {
        trace!("Initializing renderer");
        let desc = swap_chain.GetDesc().unwrap();
        let dev = swap_chain.GetDevice::<ID3D12Device>().unwrap();

        let renderer_heap: ID3D12DescriptorHeap = dev
            .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: desc.BufferCount,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })
            .unwrap();

        let command_allocator: ID3D12CommandAllocator =
            dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();

        let command_list: ID3D12GraphicsCommandList = dev
            .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
            .unwrap();
        command_list.Close().unwrap();

        command_list
            .SetName(PCWSTR(u16cstr!("hudhook Command List").as_ptr()))
            .expect("Couldn't set command list name");

        let rtv_heap: ID3D12DescriptorHeap = dev
            .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: desc.BufferCount,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 1,
            })
            .unwrap();

        let rtv_heap_inc_size =
            dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

        let rtv_handle_start = rtv_heap.GetCPUDescriptorHandleForHeapStart();
        trace!("rtv_handle_start ptr {:x}", rtv_handle_start.ptr);

        let frame_contexts: Vec<FrameContext> = (0..desc.BufferCount)
            .map(|i| {
                let desc_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_handle_start.ptr + (i * rtv_heap_inc_size) as usize,
                };
                trace!("desc handle {i} ptr {:x}", desc_handle.ptr);

                let back_buffer = swap_chain.GetBuffer(i).unwrap();
                dev.CreateRenderTargetView(&back_buffer, null(), desc_handle);

                let command_allocator: ID3D12CommandAllocator =
                    dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();
                let command_allocator_name = COMMAND_ALLOCATOR_NAMES
                    [usize::min(COMMAND_ALLOCATOR_NAMES.len() - 1, i as usize)];

                command_allocator
                    .SetName(PCWSTR(command_allocator_name.as_ptr()))
                    .expect("Couldn't set command allocator name");

                FrameContext {
                    desc_handle,
                    back_buffer,
                    command_allocator,
                    fence: dev.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap(),
                    fence_val: 0,
                    fence_event: CreateEventExW(null(), PCWSTR(null()), CREATE_EVENT(0), 0x1F0003)
                        .unwrap(),
                }
            })
            .collect();

        trace!("number of frame contexts: {}", frame_contexts.len());

        let mut ctx = Context::create();
        let cpu_desc = renderer_heap.GetCPUDescriptorHandleForHeapStart();
        let gpu_desc = renderer_heap.GetGPUDescriptorHandleForHeapStart();
        let engine = RenderEngine::new(
            &mut ctx,
            dev,
            desc.BufferCount,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            renderer_heap.clone(),
            cpu_desc,
            gpu_desc,
        );

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongPtrA(
            desc.OutputWindow,
            GWLP_WNDPROC,
            imgui_wnd_proc as usize as isize,
        ));

        #[cfg(target_arch = "x86")]
        let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongA(
            desc.OutputWindow,
            GWLP_WNDPROC,
            imgui_wnd_proc as usize as i32,
        ));

        ctx.set_ini_filename(None);

        let flags = ImguiRenderLoopFlags { focused: true };

        IMGUI_RENDER_LOOP.get_mut().unwrap().initialize(&mut ctx);

        debug!("Done init");
        let mut renderer = ImguiRenderer {
            ctx,
            command_queue: None,
            command_list,
            engine,
            wnd_proc,
            flags,
            _rtv_heap: rtv_heap,
            renderer_heap,
            frame_contexts,
            swap_chain,
        };

        ImguiWindowsEventHandler::setup_io(&mut renderer);

        renderer
    }

    fn store_swap_chain(&mut self, swap_chain: Option<IDXGISwapChain3>) -> IDXGISwapChain3 {
        if let Some(swap_chain) = swap_chain {
            self.swap_chain = swap_chain;
        }

        self.swap_chain.clone()
    }

    fn render(&mut self, swap_chain: Option<IDXGISwapChain3>) -> Option<()> {
        let render_start = Instant::now();

        let swap_chain = self.store_swap_chain(swap_chain);

        let frame_contexts_idx = unsafe { swap_chain.GetCurrentBackBufferIndex() } as usize;
        let frame_context = &mut self.frame_contexts[frame_contexts_idx];

        trace!("Rendering started");
        let sd = unsafe { swap_chain.GetDesc() }.unwrap();
        let mut rect: RECT = Default::default();

        if unsafe { GetWindowRect(sd.OutputWindow, &mut rect as _).as_bool() } {
            let mut io = self.ctx.io_mut();

            io.display_size = [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];

            let mut pos = POINT { x: 0, y: 0 };

            let active_window = unsafe { GetForegroundWindow() };
            if !HANDLE(active_window.0).is_invalid()
                && (active_window == sd.OutputWindow
                    || unsafe { IsChild(active_window, sd.OutputWindow) }.as_bool())
            {
                let gcp = unsafe { GetCursorPos(&mut pos as *mut _) };
                if gcp.as_bool()
                    && unsafe { ScreenToClient(sd.OutputWindow, &mut pos as *mut _) }.as_bool()
                {
                    io.mouse_pos[0] = pos.x as _;
                    io.mouse_pos[1] = pos.y as _;
                }
            }
        } else {
            trace!("GetWindowRect error: {:x}", unsafe { GetLastError().0 });
        }

        let command_queue = match self.command_queue.as_ref() {
            Some(cq) => cq,
            None => {
                error!("Null command queue");
                return None;
            },
        };

        self.engine.new_frame(&mut self.ctx);
        let ctx = &mut self.ctx;
        let ui = ctx.frame();
        unsafe { IMGUI_RENDER_LOOP.get_mut() }.unwrap().render(ui, &self.flags);
        let draw_data = ctx.render();

        let transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: Some(frame_context.back_buffer.clone()),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: D3D12_RESOURCE_STATE_PRESENT,
            StateAfter: D3D12_RESOURCE_STATE_RENDER_TARGET,
        });

        let mut barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 { Transition: transition_barrier },
        };

        frame_context.wait_fence();
        frame_context.incr();

        let command_allocator = &frame_context.command_allocator;

        unsafe {
            command_allocator.Reset().unwrap();
            self.command_list.Reset(command_allocator, None).unwrap();
            self.command_list.ResourceBarrier(&[barrier.clone()]);
            self.command_list.OMSetRenderTargets(
                1,
                &frame_context.desc_handle,
                BOOL::from(false),
                null(),
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.renderer_heap.clone())]);
        };

        if let Err(e) =
            self.engine.render_draw_data(draw_data, &self.command_list, frame_contexts_idx)
        {
            trace!("{}", e);
            if DXGI_DEBUG_ENABLED.load(Ordering::SeqCst) {
                unsafe { print_dxgi_debug_messages() }
            };
        };

        // Explicit auto deref necessary because this is ManuallyDrop.
        #[allow(clippy::explicit_auto_deref)]
        unsafe {
            (*barrier.Anonymous.Transition).StateBefore = D3D12_RESOURCE_STATE_RENDER_TARGET;
            (*barrier.Anonymous.Transition).StateAfter = D3D12_RESOURCE_STATE_PRESENT;
        }

        let barriers = vec![barrier];

        unsafe {
            self.command_list.ResourceBarrier(&barriers);
            self.command_list.Close().unwrap();
            command_queue.ExecuteCommandLists(&[Some(self.command_list.clone().into())]);
            command_queue.Signal(&frame_context.fence, frame_context.fence_val).unwrap();
        }

        let barrier = barriers.into_iter().next().unwrap();

        let _ = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
        trace!("Rendering done in {:?}", render_start.elapsed());
        None
    }

    unsafe fn cleanup(&mut self, swap_chain: Option<IDXGISwapChain3>) {
        let swap_chain = self.store_swap_chain(swap_chain);
        let desc = swap_chain.GetDesc().unwrap();

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        SetWindowLongPtrA(desc.OutputWindow, GWLP_WNDPROC, self.wnd_proc as usize as isize);

        #[cfg(target_arch = "x86")]
        SetWindowLongA(desc.OutputWindow, GWLP_WNDPROC, self.wnd_proc as usize as i32);
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

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.
fn get_present_addr() -> (DXGISwapChainPresentType, ExecuteCommandListsType, ResizeBuffersType) {
    let factory: IDXGIFactory = unsafe { CreateDXGIFactory() }.unwrap();
    let adapter = unsafe { factory.EnumAdapters(0) }.unwrap();

    let mut dev: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut dev) }.unwrap();
    let dev = dev.unwrap();

    let queue_desc = D3D12_COMMAND_QUEUE_DESC {
        Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
        Priority: 0,
        Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
        NodeMask: 0,
    };

    let command_queue: ID3D12CommandQueue =
        unsafe { dev.CreateCommandQueue(&queue_desc as *const _) }.unwrap();

    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            &[D3D_FEATURE_LEVEL_11_0],
            D3D11_SDK_VERSION,
            &DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                    ..Default::default()
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 1,
                OutputWindow: GetDesktopWindow(),
                Windowed: BOOL(1),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, ..Default::default() },
                ..Default::default()
            },
            &mut p_swap_chain,
            &mut p_device,
            null_mut(),
            &mut p_context,
        )
        .expect("D3D11CreateDeviceAndSwapChain failed");
    }

    let swap_chain = p_swap_chain.unwrap();

    let present_ptr = swap_chain.vtable().Present;
    let ecl_ptr = command_queue.vtable().ExecuteCommandLists;
    let rbuf_ptr = swap_chain.vtable().ResizeBuffers;

    unsafe {
        (
            std::mem::transmute(present_ptr),
            std::mem::transmute(ecl_ptr),
            std::mem::transmute(rbuf_ptr),
        )
    }
}

/// Globally enables DXGI debug messages.
pub fn enable_dxgi_debug() {
    info!("DXGI debugging enabled");
    DXGI_DEBUG_ENABLED.store(true, Ordering::SeqCst);
}

/// Globally disables DXGI debug messages.
pub fn disable_dxgi_debug() {
    info!("DXGI debugging disabled");
    DXGI_DEBUG_ENABLED.store(false, Ordering::SeqCst);
}

/// Stores hook detours and implements the [`Hooks`] trait.
pub struct ImguiDx12Hooks(MhHooks);

impl ImguiDx12Hooks {
    /// Construct a set of [`RawDetour`]s that will render UI via the provided
    /// [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDXGISwapChain::Present`
    /// - `IDXGISwapChain::ResizeBuffers`
    /// - `ID3D12CommandQueue::ExecuteCommandLists`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T: 'static>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync,
    {
        let (dxgi_swap_chain_present_addr, execute_command_lists_addr, resize_buffers_addr) =
            get_present_addr();

        trace!(
            "IDXGISwapChain::Present                 = {:p}",
            dxgi_swap_chain_present_addr as *const c_void
        );
        trace!(
            "ID3D12CommandQueue::ExecuteCommandLists = {:p}",
            execute_command_lists_addr as *const c_void
        );
        trace!(
            "IDXGISwapChain::ResizeBuffers            = {:p}",
            resize_buffers_addr as *const c_void
        );

        let hook_dscp = MhHook::new(
            dxgi_swap_chain_present_addr as *mut _,
            imgui_dxgi_swap_chain_present_impl as *mut _,
        )
        .expect("couldn't create IDXGISwapChain::Present hook");

        let hook_cqecl = MhHook::new(
            execute_command_lists_addr as *mut _,
            imgui_execute_command_lists_impl as *mut _,
        )
        .expect("couldn't create ID3D12CommandQueue::ExecuteCommandLists hook");

        let hook_rbuf =
            MhHook::new(resize_buffers_addr as *mut _, imgui_resize_buffers_impl as *mut _)
                .expect("couldn't create IDXGISwapChain::ResizeBuffers hook");

        IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINE.get_or_init(|| {
            (
                mem::transmute(hook_dscp.trampoline()),
                mem::transmute(hook_cqecl.trampoline()),
                mem::transmute(hook_rbuf.trampoline()),
            )
        });

        Self(MhHooks::new([hook_dscp, hook_cqecl, hook_rbuf]).expect("couldn't create hooks"))
    }
}

impl Hooks for ImguiDx12Hooks {
    unsafe fn hook(&self) {
        self.0.apply();
    }

    unsafe fn unhook(&mut self) {
        trace!("Disabling hooks...");
        self.0.unapply();

        CQECL_RUNNING.wait();
        PRESENT_RUNNING.wait();
        RBUF_RUNNING.wait();

        trace!("Cleaning up renderer...");
        if let Some(renderer) = IMGUI_RENDERER.take() {
            let mut renderer = renderer.lock();
            // XXX
            // This is a hack for solving this concurrency issue:
            // https://github.com/veeenu/hudhook/issues/34
            // We should investigate deeper into this and find a way of synchronizing with
            // the moment the actual resources involved in the rendering are
            // dropped. Using a condvar like above does not work, and still
            // leads clients to crash.
            //
            // The 34ms value was chosen because it's a bit more than 1 frame @ 30fps.
            thread::sleep(Duration::from_millis(34));
            renderer.cleanup(None);
        }

        drop(IMGUI_RENDER_LOOP.take());
        COMMAND_QUEUE_GUARD.take();

        DXGI_DEBUG_ENABLED.store(false, Ordering::SeqCst);
    }
}

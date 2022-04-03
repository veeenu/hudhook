use crate::mh;

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use std::ptr::{null, null_mut};

use log::*;
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::Mutex;
use winapi::shared::minwindef::{HMODULE, LOWORD};
use winapi::um::winuser::{GET_WHEEL_DELTA_WPARAM, GET_XBUTTON_WPARAM};
use windows::core::{IUnknown, Interface, HRESULT, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory, IDXGIFactory, IDXGISwapChain, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, *,
};
use windows::Win32::Graphics::Gdi::{ScreenToClient, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::*;

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain3, SyncInterval: u32, Flags: u32) -> HRESULT;

type ExecuteCommandListsType = unsafe extern "system" fn(
    This: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
);

type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Data structures and traits
////////////////////////////////////////////////////////////////////////////////////////////////////

trait Renderer {
    /// Invoked once per frame.
    fn render(&mut self);
}

/// Implement your `imgui` rendering logic via this trait.
pub trait ImguiRenderLoop {
    fn render(&mut self, ui: &mut imgui_dx12::imgui::Ui, flags: &ImguiRenderLoopFlags);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global singletons
////////////////////////////////////////////////////////////////////////////////////////////////////

static TRAMPOLINE: OnceCell<(DXGISwapChainPresentType, ExecuteCommandListsType)> = OnceCell::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hook entry points
////////////////////////////////////////////////////////////////////////////////////////////////////

static mut IMGUI_RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();
static IMGUI_RENDERER: OnceCell<Mutex<Box<ImguiRenderer>>> = OnceCell::new();

#[derive(Debug)]
struct FrameContext {
    back_buffer: ID3D12Resource,
    desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    command_allocator: ID3D12CommandAllocator,
}

unsafe extern "system" fn imgui_execute_command_lists_impl(
    cmd_queue: ID3D12CommandQueue,
    num_command_lists: u32,
    command_lists: *mut ID3D12CommandList,
) {
    static CELL: OnceCell<()> = OnceCell::new();

    CELL.get_or_try_init(|| {
        trace!("cmd_queue ptr is {:?}", cmd_queue);
        if let Some(renderer) = IMGUI_RENDERER.get() {
            trace!("cmd_queue ptr was set");
            renderer.lock().command_queue = Some(cmd_queue.clone());
            Ok(())
        } else {
            trace!("cmd_queue ptr was not set");
            Err(())
        }
    })
    .ok();

    let (_, trampoline) = TRAMPOLINE
        .get()
        .expect("ID3D12CommandQueue::ExecuteCommandLists trampoline uninitialized");
    trampoline(cmd_queue, num_command_lists, command_lists);
}

unsafe extern "system" fn imgui_dxgi_swap_chain_present_impl(
    swap_chain: IDXGISwapChain3,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let (trampoline_present, _) = TRAMPOLINE
        .get()
        .expect("IDXGISwapChain::Present trampoline uninitialized");

    let mut renderer = IMGUI_RENDERER
        .get_or_init(|| Mutex::new(Box::new(ImguiRenderer::new(swap_chain.clone()))))
        .lock();

    renderer.render(swap_chain.clone());
    drop(renderer);

    trampoline_present(swap_chain, sync_interval, flags)
}

unsafe extern "system" fn imgui_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
) -> LRESULT {
    match IMGUI_RENDERER.get().map(Mutex::try_lock) {
        Some(Some(mut imgui_renderer)) => {
            let ctx = &mut imgui_renderer.ctx;
            let mut io = ctx.io_mut();

            match umsg {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if wparam < 256 {
                        io.keys_down[wparam] = true;
                    }
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    if wparam < 256 {
                        io.keys_down[wparam] = false;
                    }
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    io.mouse_down[0] = true;
                    // return 1;
                }
                WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
                    // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    io.mouse_down[1] = true;
                    // return 1;
                }
                WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
                    // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    io.mouse_down[2] = true;
                    // return 1;
                }
                WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
                    let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1.0 as u16 {
                        3
                    } else {
                        4
                    };
                    // set_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    io.mouse_down[btn] = true;
                    // return 1;
                }
                WM_LBUTTONUP => {
                    io.mouse_down[0] = false;
                    // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    // return 1;
                }
                WM_RBUTTONUP => {
                    io.mouse_down[1] = false;
                    // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    // return 1;
                }
                WM_MBUTTONUP => {
                    io.mouse_down[2] = false;
                    // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                    // return 1;
                }
                WM_XBUTTONUP => {
                    let btn = if GET_XBUTTON_WPARAM(wparam) == XBUTTON1.0 as u16 {
                        3
                    } else {
                        4
                    };
                    io.mouse_down[btn] = false;
                    // release_capture(&hook.imgui_ctx.io().mouse_down, hwnd);
                }
                WM_MOUSEWHEEL => {
                    io.mouse_wheel +=
                        (GET_WHEEL_DELTA_WPARAM(wparam) as f32) / (WHEEL_DELTA as f32);
                }
                WM_MOUSEHWHEEL => {
                    io.mouse_wheel_h +=
                        (GET_WHEEL_DELTA_WPARAM(wparam) as f32) / (WHEEL_DELTA as f32);
                }
                WM_CHAR => io.add_input_character(wparam as u8 as char),
                WM_ACTIVATE => {
                    if LOWORD(wparam as _) == WA_INACTIVE as u16 {
                        imgui_renderer.flags.focused = false;
                    } else {
                        imgui_renderer.flags.focused = true;
                    }
                    return LRESULT(1);
                }
                _ => {}
            }

            let wnd_proc = imgui_renderer.wnd_proc;
            drop(imgui_renderer);

            CallWindowProcW(Some(wnd_proc), hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        }
        Some(None) => {
            debug!("Could not lock in WndProc");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        }
        None => {
            debug!("WndProc called before hook was set");
            DefWindowProcW(hwnd, umsg, WPARAM(wparam), LPARAM(lparam))
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Render loops
////////////////////////////////////////////////////////////////////////////////////////////////////

struct ImguiRenderer {
    ctx: imgui_dx12::imgui::Context,
    engine: imgui_dx12::RenderEngine,
    render_loop: Box<dyn ImguiRenderLoop>,
    wnd_proc: WndProcType,
    flags: ImguiRenderLoopFlags,
    frame_contexts: Vec<FrameContext>,
    // frame_contexts_idx: usize,
    _rtv_heap: ID3D12DescriptorHeap,
    renderer_heap: ID3D12DescriptorHeap,
    command_queue: Option<ID3D12CommandQueue>,
    command_list: ID3D12GraphicsCommandList,
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

        let command_allocator: ID3D12CommandAllocator = dev
            .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
            .unwrap();

        let command_list: ID3D12GraphicsCommandList = dev
            .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
            .unwrap();
        command_list.Close().unwrap();

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
        debug!("rtv_handle_start ptr {:x}", rtv_handle_start.ptr);

        let frame_contexts: Vec<FrameContext> = (0..desc.BufferCount)
            .map(|i| {
                let desc_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_handle_start.ptr + (i * rtv_heap_inc_size) as usize,
                };
                debug!("desc handle {i} ptr {:x}", desc_handle.ptr);
                let back_buffer = swap_chain.GetBuffer(i).unwrap();
                dev.CreateRenderTargetView(&back_buffer, null(), desc_handle);
                FrameContext {
                    desc_handle,
                    back_buffer,
                    command_allocator: dev
                        .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                        .unwrap(),
                }
            })
            .collect();

        let mut ctx = imgui::Context::create();
        let cpu_desc = renderer_heap.GetCPUDescriptorHandleForHeapStart();
        let gpu_desc = renderer_heap.GetGPUDescriptorHandleForHeapStart();
        let engine = imgui_dx12::RenderEngine::new(
            &mut ctx,
            dev,
            desc.BufferCount,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            renderer_heap.clone(),
            cpu_desc,
            gpu_desc,
        );
        let render_loop = IMGUI_RENDER_LOOP.take().unwrap();
        let wnd_proc = std::mem::transmute::<_, WndProcType>(SetWindowLongPtrA(
            desc.OutputWindow,
            GWLP_WNDPROC,
            imgui_wnd_proc as usize as isize,
        ));

        ctx.set_ini_filename(None);
        ctx.io_mut().nav_active = true;
        ctx.io_mut().nav_visible = true;

        let flags = ImguiRenderLoopFlags { focused: true };

        debug!("Done init");
        ImguiRenderer {
            ctx,
            command_queue: None,
            command_list,
            engine,
            render_loop,
            wnd_proc,
            flags,
            _rtv_heap: rtv_heap,
            renderer_heap,
            frame_contexts,
        }
    }

    fn render(&mut self, swap_chain: IDXGISwapChain3) -> Option<()> {
        let sd = unsafe { swap_chain.GetDesc() }.unwrap();
        let mut rect: RECT = Default::default();

        if unsafe { GetWindowRect(sd.OutputWindow, &mut rect as _).as_bool() } {
            let mut io = self.ctx.io_mut();

            io.display_size = [
                (rect.right - rect.left) as f32,
                (rect.bottom - rect.top) as f32,
            ];

            let mut pos = POINT { x: 0, y: 0 };

            let active_window = unsafe { GetForegroundWindow() };
            if !active_window.is_invalid()
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
        }

        let command_queue = self.command_queue.as_ref()?;

        let frame_contexts_idx = unsafe { swap_chain.GetCurrentBackBufferIndex() } as usize;
        let frame_context = &self.frame_contexts[frame_contexts_idx];

        self.engine.new_frame(&mut self.ctx);
        let ctx = &mut self.ctx;
        let mut ui = ctx.frame();
        self.render_loop.render(&mut ui, &self.flags);
        let draw_data = ui.render();

        let mut transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: Some(frame_context.back_buffer.clone()),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: D3D12_RESOURCE_STATE_PRESENT,
            StateAfter: D3D12_RESOURCE_STATE_RENDER_TARGET,
        });

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: transition_barrier.clone(),
            },
        };

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
            self.command_list
                .SetDescriptorHeaps(&[Some(self.renderer_heap.clone())]);
        };

        self.engine
            .render_draw_data(draw_data, &self.command_list, frame_contexts_idx);
        transition_barrier.StateBefore = D3D12_RESOURCE_STATE_RENDER_TARGET;
        transition_barrier.StateAfter = D3D12_RESOURCE_STATE_PRESENT;

        unsafe {
            self.command_list.ResourceBarrier(&[barrier]);
            self.command_list.Close().unwrap();
            command_queue.ExecuteCommandLists(&[Some(self.command_list.clone().into())]);
        }

        drop(transition_barrier);
        None
    }
}

unsafe impl Send for ImguiRenderer {}
unsafe impl Sync for ImguiRenderer {}

/// Holds information useful to the render loop which can't be retrieved from `imgui::Ui`.
pub struct ImguiRenderLoopFlags {
    /// Whether the hooked program's window is currently focused.
    pub focused: bool,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Function address finders
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Get the `IDXGISwapChain::Present` function address.
///
/// Creates a swap chain + device instance and looks up its
/// vtable to find the address.
fn get_present_addr() -> (DXGISwapChainPresentType, ExecuteCommandListsType) {
    trace!("get_present_addr");
    trace!("  HWND");
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcA(hwnd, msg, wparam, lparam)
    }
    let hinstance = unsafe { GetModuleHandleA(None) };
    let hwnd = {
        let wnd_class = WNDCLASSEXA {
            style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
            cbClsExtra: 0,
            cbWndExtra: 0,
            cbSize: size_of::<WNDCLASSEXA>() as u32,
            hIcon: HICON(0),
            hIconSm: HICON(0),
            hCursor: HCURSOR(0),
            hbrBackground: HBRUSH(0),
            lpszMenuName: PCSTR(null()),
        };
        unsafe {
            trace!("    RegisterClassExA");
            RegisterClassExA(&wnd_class);
            trace!("    CreateWindowExA");
            CreateWindowExA(
                WINDOW_EX_STYLE(0),
                PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
                PCSTR("HUDHOOK_DUMMY\0".as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                0,
                0,
                100,
                100,
                None,
                None,
                hinstance,
                null(),
            )
        }
    };

    let factory: IDXGIFactory = unsafe { CreateDXGIFactory() }.unwrap();
    let adapter = unsafe { factory.EnumAdapters(0) }.unwrap();

    let mut dev: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut dev) }.unwrap();
    let dev = dev.unwrap();

    let mut queue_desc = D3D12_COMMAND_QUEUE_DESC::default();
    queue_desc.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
    queue_desc.Priority = 0;
    queue_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
    queue_desc.NodeMask = 0;

    let command_queue: ID3D12CommandQueue =
        unsafe { dev.CreateCommandQueue(&queue_desc as *const _) }.unwrap();
    let command_alloc: ID3D12CommandAllocator =
        unsafe { dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.unwrap();
    let command_list: ID3D12CommandList =
        unsafe { dev.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_alloc, None) }
            .unwrap();

    let swap_chain_desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: 100,
            Height: 100,
            RefreshRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
            Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
        },
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        OutputWindow: hwnd,
        Windowed: BOOL::from(true),
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
    };

    let swap_chain = unsafe { factory.CreateSwapChain(&command_queue, &swap_chain_desc) }.unwrap();
    let present_ptr = unsafe { swap_chain.vtable().Present };
    let ecl_ptr = unsafe { command_queue.vtable().ExecuteCommandLists };

    unsafe { DestroyWindow(hwnd) };
    unsafe { UnregisterClassA(PCSTR("HUDHOOK_DUMMY\0".as_ptr()), hinstance) };

    unsafe {
        (
            std::mem::transmute(present_ptr),
            std::mem::transmute(ecl_ptr),
        )
    }
}

/// Construct a `mh::Hook` that will render UI via the provided `ImguiRenderLoop`.
///
/// # Safety
///
/// yolo
pub unsafe fn hook_imgui<T: 'static>(t: T) -> [mh::Hook; 2]
where
    T: ImguiRenderLoop + Send + Sync,
{
    let (dxgi_swap_chain_present_addr, execute_command_lists_addr) = get_present_addr();
    debug!(
        "IDXGISwapChain::Present = {:p}",
        dxgi_swap_chain_present_addr as *const c_void
    );
    debug!(
        "ID3D12CommandQueue::ExecuteCommandLists = {:p}",
        execute_command_lists_addr as *const c_void
    );

    let mut trampoline_dscp = null_mut();
    let mut trampoline_cqecl = null_mut();

    debug!(
        "MH_CreateHook: {:?}",
        mh::MH_CreateHook(
            dxgi_swap_chain_present_addr as *mut c_void,
            imgui_dxgi_swap_chain_present_impl as *mut c_void,
            &mut trampoline_dscp as *mut _ as _
        )
    );
    debug!(
        "MH_CreateHook: {:?}",
        mh::MH_CreateHook(
            execute_command_lists_addr as *mut c_void,
            imgui_execute_command_lists_impl as *mut c_void,
            &mut trampoline_cqecl as *mut _ as _
        )
    );

    IMGUI_RENDER_LOOP.get_or_init(|| Box::new(t));
    TRAMPOLINE.get_or_init(|| {
        (
            std::mem::transmute(trampoline_dscp),
            std::mem::transmute(trampoline_cqecl),
        )
    });

    [
        mh::Hook::new(
            dxgi_swap_chain_present_addr as *mut c_void,
            imgui_dxgi_swap_chain_present_impl as *mut c_void,
        ),
        mh::Hook::new(
            execute_command_lists_addr as *mut c_void,
            imgui_execute_command_lists_impl as *mut c_void,
        ),
    ]
}

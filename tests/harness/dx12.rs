use std::ffi::CString;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};

use hudhook::renderer::{enable_debug_interface, print_dxgi_debug_messages};
use hudhook::util;
use tracing::{error, trace};
use windows::core::{s, ComInterface, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12DescriptorHeap,
    ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
    D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
    D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET,
    D3D12_RESOURCE_TRANSITION_BARRIER,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain, IDXGISwapChain3, DXGI_CREATE_FACTORY_DEBUG,
    DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::{
    CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT, INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, PeekMessageA,
    PostQuitMessage, RegisterClassA, SetTimer, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    HCURSOR, HICON, HMENU, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WM_SIZE, WNDCLASSA,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

static RESIZE: OnceLock<Sender<(u32, u32)>> = OnceLock::new();

pub struct Barrier;

impl Barrier {
    pub fn create(
        buf: ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) -> [D3D12_RESOURCE_BARRIER; 1] {
        let transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: ManuallyDrop::new(Some(buf)),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: before,
            StateAfter: after,
        });

        [D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 { Transition: transition_barrier },
        }]
    }

    pub fn drop(barrier: [D3D12_RESOURCE_BARRIER; 1]) {
        for barrier in barrier {
            let transition = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
            let _ = ManuallyDrop::into_inner(transition.pResource);
        }
    }
}

pub struct Dx12Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
    _caption: Arc<CString>,
}

impl Dx12Harness {
    #[allow(unused)]
    pub fn new(caption: &str) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let caption = Arc::new(CString::new(caption).unwrap());
        let child = Some(thread::spawn({
            let done = Arc::clone(&done);
            let caption = Arc::clone(&caption);

            move || {
                let hinstance = unsafe { GetModuleHandleA(PCSTR(null())).unwrap() };
                let wnd_class = WNDCLASSA {
                    style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: s!("MyClass\0"),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hIcon: HICON::default(),
                    hCursor: HCURSOR::default(),
                    hbrBackground: HBRUSH::default(),
                    lpszMenuName: PCSTR(null()),
                };
                unsafe { RegisterClassA(&wnd_class) };
                let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
                unsafe {
                    AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, BOOL::from(false))
                };
                let hwnd = unsafe {
                    CreateWindowExA(
                        WINDOW_EX_STYLE::default(),
                        PCSTR("MyClass\0".as_ptr()),
                        PCSTR(caption.as_ptr().cast()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
                        // size and position
                        100,
                        100,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        HWND::default(),  // hWndParent
                        HMENU::default(), // hMenu
                        hinstance,        // hInstance
                        None,
                    )
                }; // lpParam

                unsafe { enable_debug_interface() };

                let factory: IDXGIFactory2 =
                    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG) }.unwrap();
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
                let command_alloc: ID3D12CommandAllocator =
                    unsafe { dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.unwrap();
                let command_list: ID3D12GraphicsCommandList = unsafe {
                    dev.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_alloc, None)
                }
                .unwrap();
                unsafe { command_list.Close().unwrap() };

                let swap_chain_desc = DXGI_SWAP_CHAIN_DESC {
                    BufferDesc: DXGI_MODE_DESC {
                        Width: 800,
                        Height: 600,
                        RefreshRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                        Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                    },
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    OutputWindow: hwnd,
                    Windowed: BOOL::from(true),
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
                };

                let mut swap_chain: Option<IDXGISwapChain> = None;
                unsafe {
                    factory.CreateSwapChain(
                        &command_queue,
                        &swap_chain_desc,
                        &mut swap_chain as *mut _,
                    )
                }
                .unwrap();
                let swap_chain: IDXGISwapChain3 = swap_chain.unwrap().cast().unwrap();
                let desc = unsafe {
                    let mut desc = Default::default();
                    swap_chain.GetDesc(&mut desc).unwrap();
                    desc
                };

                let _renderer_heap: ID3D12DescriptorHeap = unsafe {
                    dev.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        NumDescriptors: desc.BufferCount,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
                    .unwrap()
                };

                let rtv_heap: ID3D12DescriptorHeap = unsafe {
                    dev.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                        NumDescriptors: desc.BufferCount,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                        NodeMask: 1,
                    })
                    .unwrap()
                };

                let mut rtv_handles = Vec::new();
                let rtv_heap_inc_size =
                    unsafe { dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) };
                let rtv_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

                for i in 0..desc.BufferCount {
                    unsafe {
                        let buf: ID3D12Resource = swap_chain.GetBuffer(i).unwrap();
                        let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                            ptr: rtv_start.ptr + (i * rtv_heap_inc_size) as usize,
                        };
                        dev.CreateRenderTargetView(&buf, None, rtv_handle);
                        rtv_handles.push(rtv_handle);
                    }
                }

                unsafe { SetTimer(hwnd, 0, 100, None) };

                let fence: ID3D12Fence =
                    unsafe { dev.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap() };
                let mut fence_val = 0;
                let fence_event =
                    unsafe { CreateEventExW(None, None, CREATE_EVENT(0), 0x1F0003) }.unwrap();

                let (tx, rx) = mpsc::channel();

                RESIZE.get_or_init(move || tx);

                loop {
                    trace!("Debug");
                    unsafe {
                        print_dxgi_debug_messages();
                    }

                    trace!("Clearing");
                    unsafe {
                        let rtv_barrier = Barrier::create(
                            swap_chain.GetBuffer(swap_chain.GetCurrentBackBufferIndex()).unwrap(),
                            D3D12_RESOURCE_STATE_PRESENT,
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                        );
                        let present_barrier = Barrier::create(
                            swap_chain.GetBuffer(swap_chain.GetCurrentBackBufferIndex()).unwrap(),
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                            D3D12_RESOURCE_STATE_PRESENT,
                        );

                        let rtv = rtv_handles[swap_chain.GetCurrentBackBufferIndex() as usize];

                        command_alloc.Reset().unwrap();
                        command_list.Reset(&command_alloc, None).unwrap();
                        trace!("RB");
                        command_list.ResourceBarrier(&rtv_barrier);
                        trace!("ClearRTV");
                        command_list.ClearRenderTargetView(rtv, &[0.3, 0.8, 0.3, 0.8], None);
                        trace!("RB");
                        command_list.ResourceBarrier(&present_barrier);
                        command_list.Close().unwrap();
                        trace!("ECL");
                        command_queue.ExecuteCommandLists(&[Some(command_list.cast().unwrap())]);
                        trace!("ECL done");
                        command_queue.Signal(&fence, fence_val);
                        trace!("Signal done");

                        trace!("Present");
                        if let Err(e) = swap_chain.Present(1, 0).ok() {
                            if let Err(e) = dev.GetDeviceRemovedReason() {
                                error!("Device removed: {e:?}");
                            }
                            print_dxgi_debug_messages();
                            panic!("{e:?}");
                        }

                        if fence.GetCompletedValue() < fence_val {
                            fence.SetEventOnCompletion(fence_val, fence_event);
                            WaitForSingleObjectEx(fence_event, INFINITE, false);
                        }

                        fence_val += 1;

                        Barrier::drop(present_barrier);
                        Barrier::drop(rtv_barrier);
                    }

                    trace!("Handle message");
                    if !handle_message(hwnd) {
                        break;
                    }

                    trace!("Resize");
                    if let Some((width, height)) = rx.try_iter().last() {
                        let desc =
                            util::try_out_param(|v| unsafe { swap_chain.GetDesc(v) }).unwrap();
                        unsafe {
                            swap_chain.ResizeBuffers(
                                desc.BufferCount,
                                width,
                                height,
                                DXGI_FORMAT_B8G8R8A8_UNORM,
                                0,
                            )
                        };

                        for i in 0..desc.BufferCount {
                            unsafe {
                                let buf: ID3D12Resource = swap_chain.GetBuffer(i).unwrap();
                                let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                                    ptr: rtv_start.ptr + (i * rtv_heap_inc_size) as usize,
                                };
                                dev.CreateRenderTargetView(&buf, None, rtv_handle);
                                rtv_handles[i as usize] = rtv_handle;
                            }
                        }
                    };

                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }));

        Self { child, done, _caption: caption }
    }
}

impl Drop for Dx12Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
    }
}

#[allow(unused)]
fn handle_message(window: HWND) -> bool {
    unsafe {
        let mut msg = MaybeUninit::uninit();
        if PeekMessageA(msg.as_mut_ptr(), window, 0, 0, PM_REMOVE).as_bool() {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr().as_ref().map(|m| m.message != WM_QUIT).unwrap_or(true)
        } else {
            true
        }
    }
}

#[allow(unused)]
pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
        },
        WM_SIZE => {
            let (width, height) = hudhook::util::win_size(hwnd);
            if let Some(tx) = RESIZE.get() {
                tx.send((width as _, height as _));
            }
        },
        _ => {
            return DefWindowProcA(hwnd, msg, w_param, l_param);
        },
    }
    LRESULT(0)
}

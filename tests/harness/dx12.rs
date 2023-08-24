use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use tracing::trace;
use windows::core::{s, ComInterface, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, D3D12GetDebugInterface, ID3D12CommandAllocator, ID3D12CommandQueue,
    ID3D12Debug, ID3D12DescriptorHeap, ID3D12Device, ID3D12GraphicsCommandList, ID3D12Resource,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
    D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory, DXGIGetDebugInterface1, IDXGIFactory, IDXGIInfoQueue, IDXGISwapChain,
    IDXGISwapChain3, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
    PostQuitMessage, RegisterClassA, SetTimer, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    HCURSOR, HICON, HMENU, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

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

                let mut debug_interface: Option<ID3D12Debug> = None;
                unsafe { D3D12GetDebugInterface(&mut debug_interface) }.unwrap();
                unsafe { debug_interface.as_ref().unwrap().EnableDebugLayer() };

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
                let command_alloc: ID3D12CommandAllocator =
                    unsafe { dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.unwrap();
                let command_list: ID3D12GraphicsCommandList = unsafe {
                    dev.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_alloc, None)
                }
                .unwrap();

                let swap_chain_desc = DXGI_SWAP_CHAIN_DESC {
                    BufferDesc: DXGI_MODE_DESC {
                        Width: 800,
                        Height: 600,
                        RefreshRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
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
                    }
                }

                let diq: IDXGIInfoQueue = unsafe { DXGIGetDebugInterface1(0) }.unwrap();

                unsafe { SetTimer(hwnd, 0, 100, None) };

                loop {
                    trace!("Debug");
                    unsafe {
                        for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                            let mut msg_len: usize = 0;
                            diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap();
                            let diqm = vec![0u8; msg_len];
                            let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                            diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _)
                                .unwrap();
                            let diqm = pdiqm.as_ref().unwrap();
                            println!(
                                "{}",
                                String::from_utf8_lossy(std::slice::from_raw_parts(
                                    diqm.pDescription,
                                    diqm.DescriptionByteLength
                                ))
                            );
                        }
                        diq.ClearStoredMessages(DXGI_DEBUG_ALL);
                    }

                    unsafe {
                        command_list.Close().unwrap();
                        command_alloc.Reset().unwrap();
                        command_list.Reset(&command_alloc, None).unwrap();
                    }

                    trace!("Present");
                    unsafe { swap_chain.Present(1, 0) }.unwrap();

                    trace!("Handle message");
                    if !handle_message(hwnd) {
                        break;
                    }

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
        if GetMessageA(msg.as_mut_ptr(), window, 0, 0).as_bool() {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr().as_ref().map(|m| m.message != WM_QUIT).unwrap_or(true)
        } else {
            false
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
        _ => {
            return DefWindowProcA(hwnd, msg, w_param, l_param);
        },
    }
    LRESULT(0)
}

use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use hudhook::util;
use once_cell::sync::OnceCell;
use tracing::{error, trace};
use windows::core::{w, Interface, Result, PCSTR, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

type Msg = (HWND, u32, WPARAM, LPARAM);
static TX: OnceCell<Arc<Sender<Msg>>> = OnceCell::new();

pub struct Dx12Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
}

impl Dx12Harness {
    #[allow(unused)]
    pub fn new() -> Self {
        let done = Arc::new(AtomicBool::new(false));

        let child = Some(thread::spawn({
            let done = Arc::clone(&done);

            let (tx, rx) = mpsc::channel();
            TX.get_or_init(move || Arc::new(tx));

            move || unsafe {
                if let Err(e) = run_harness(done, rx) {
                    util::print_dxgi_debug_messages();
                    error!("{e:?}");
                }
            }
        }));

        Self { child, done }
    }
}

impl Drop for Dx12Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
    }
}

unsafe fn run_harness(
    done: Arc<AtomicBool>,
    rx: Receiver<(HWND, u32, WPARAM, LPARAM)>,
) -> Result<()> {
    trace!("Creating window");
    let hinstance = GetModuleHandleA(PCSTR(null())).unwrap();
    let wnd_class = WNDCLASSW {
        style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance.into(),
        lpszClassName: w!("MyClass"),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCWSTR(null()),
    };
    RegisterClassW(&wnd_class);

    let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
    AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, BOOL::from(false))?;

    trace!("a");
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("MyClass"),
        w!("Dx12 hook example"),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
        100,
        100,
        rect.right - rect.left,
        rect.bottom - rect.top,
        HWND::default(),
        HMENU::default(),
        hinstance,
        None,
    );

    trace!("Enabling debug");
    util::enable_debug_interface();

    let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG)?;
    let adapter = factory.EnumAdapters(0)?;

    let device: ID3D12Device =
        util::try_out_ptr(|v| D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, v))?;

    let command_queue: ID3D12CommandQueue =
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        })?;

    let command_allocator: ID3D12CommandAllocator =
        device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;

    let command_list: ID3D12GraphicsCommandList =
        device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)?;

    command_list.Close()?;

    command_queue.SetName(w!("Harness Command Queue"))?;
    command_allocator.SetName(w!("Harness Command Allocator"))?;
    command_list.SetName(w!("Harness Command List"))?;

    let swap_chain: IDXGISwapChain3 = factory
        .CreateSwapChainForHwnd(
            &command_queue,
            hwnd,
            &DXGI_SWAP_CHAIN_DESC1 {
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                Flags: 0,
                Width: 800,
                Height: 600,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                Scaling: DXGI_SCALING_NONE,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            },
            None,
            None,
        )?
        .cast()?;

    drop(adapter);
    drop(factory);

    let rtv_heap: ID3D12DescriptorHeap =
        device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: 2,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
            NodeMask: 0,
        })?;

    let rtv_desc0 = rtv_heap.GetCPUDescriptorHandleForHeapStart();
    let rtv_desc1 = D3D12_CPU_DESCRIPTOR_HANDLE {
        ptr: rtv_desc0.ptr
            + device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) as usize,
    };

    {
        let buf: ID3D12Resource = swap_chain.GetBuffer(0).unwrap();
        buf.SetName(w!("Harness back buffer 0"))?;
        device.CreateRenderTargetView(&buf, None, rtv_desc0);
        drop(buf);
        let buf: ID3D12Resource = swap_chain.GetBuffer(1).unwrap();
        buf.SetName(w!("Harness back buffer 1"))?;
        device.CreateRenderTargetView(&buf, None, rtv_desc1);
        drop(buf);
    }

    let rtv = [rtv_desc0, rtv_desc1];

    let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
    let mut fence_val = 0u64;
    let fence_event = CreateEventExW(None, None, CREATE_EVENT(0), 0x1F0003)?;

    loop {
        util::print_dxgi_debug_messages();
        let rtv = rtv[swap_chain.GetCurrentBackBufferIndex() as usize];
        let back_buffer = swap_chain.GetBuffer(swap_chain.GetCurrentBackBufferIndex())?;

        let rtv_barrier = [util::create_barrier(
            &back_buffer,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        )];

        let present_barrier = [util::create_barrier(
            &back_buffer,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        )];

        drop(back_buffer);

        command_allocator.Reset()?;
        command_list.Reset(&command_allocator, None)?;
        command_list.ResourceBarrier(&rtv_barrier);
        command_list.ClearRenderTargetView(rtv, &[0.3, 0.8, 0.3, 0.8], None);
        command_list.ResourceBarrier(&present_barrier);
        command_list.Close()?;
        command_queue.ExecuteCommandLists(&[Some(command_list.cast()?)]);
        command_queue.Signal(&fence, fence_val)?;

        if fence.GetCompletedValue() < fence_val {
            fence.SetEventOnCompletion(fence_val, fence_event)?;
            WaitForSingleObject(fence_event, INFINITE);
        }
        fence_val += 1;

        rtv_barrier.into_iter().for_each(util::drop_barrier);
        present_barrier.into_iter().for_each(util::drop_barrier);

        swap_chain.Present(0, 0).ok()?;

        let mut msg = MSG::default();
        if PeekMessageA(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }

        for msg in rx.try_iter() {
            let (_hwnd, msg, _wparam, lparam) = msg;

            match msg {
                WM_DESTROY => {
                    PostQuitMessage(0);
                    break;
                },
                WM_SIZE => {
                    let width = loword(lparam.0 as u32) as u32;
                    let height = hiword(lparam.0 as u32) as u32;
                    trace!("Resizing {width}x{height}");

                    // TODO look deeper into this crash.
                    // swap_chain.ResizeBuffers(2, width, height, DXGI_FORMAT_B8G8R8A8_UNORM, 0)?;
                    trace!("Resized");

                    let buf: ID3D12Resource = swap_chain.GetBuffer(0).unwrap();
                    buf.SetName(w!("Harness back buffer 0"))?;
                    device.CreateRenderTargetView(&buf, None, rtv_desc0);
                    drop(buf);

                    let buf: ID3D12Resource = swap_chain.GetBuffer(1).unwrap();
                    buf.SetName(w!("Harness back buffer 1"))?;
                    device.CreateRenderTargetView(&buf, None, rtv_desc1);
                    drop(buf);
                },
                _ => {},
            }
        }

        if done.load(Ordering::SeqCst) {
            break;
        }
    }

    Ok(())
}

// Replication of the Win32 HIWORD macro.
#[inline]
pub fn hiword(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

// Replication of the Win32 LOWORD macro.
#[inline]
pub fn loword(l: u32) -> u16 {
    (l & 0xffff) as u16
}

pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(tx) = TX.get() {
        tx.send((hwnd, msg, wparam, lparam)).ok();
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

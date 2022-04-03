// Mandatory reference:
// https://www.codeslow.com/2019/12/tiny-windows-executable-in-rust.html

#![no_main]

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr::{null, null_mut, NonNull};

use imgui::{Condition, Window};
use imgui_dx12::RenderEngine;
use log::{info, trace};
use windows::core::{IUnknown, Interface, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::*;

#[no_mangle]
pub fn main(_argc: i32, _argv: *const *const u8) {
    use simplelog::*;
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Trace,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Trace,
            Config::default(),
            std::fs::File::create(Path::new("eldenring-practice-tool.log")).unwrap(),
        ),
    ])
    .ok();

    let hinstance = unsafe { GetModuleHandleA(PCSTR(null())) };
    let wnd_class = WNDCLASSA {
        style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: PCSTR("MyClass\0".as_ptr()),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCSTR(null()),
    };
    unsafe { RegisterClassA(&wnd_class) };
    let hwnd = unsafe {
        CreateWindowExA(
            WINDOW_EX_STYLE::default(),
            PCSTR("MyClass\0".as_ptr()),
            PCSTR("MiniWIN\0".as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
            // size and position
            100,
            100,
            800,
            600,
            HWND::default(),  // hWndParent
            HMENU::default(), // hMenu
            hinstance,        // hInstance
            null(),
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

    let mut queue_desc = D3D12_COMMAND_QUEUE_DESC::default();
    queue_desc.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
    queue_desc.Priority = 0;
    queue_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
    queue_desc.NodeMask = 0;

    let command_queue: ID3D12CommandQueue =
        unsafe { dev.CreateCommandQueue(&queue_desc as *const _) }.unwrap();
    let command_alloc: ID3D12CommandAllocator =
        unsafe { dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.unwrap();
    let command_list: ID3D12GraphicsCommandList =
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

    let swap_chain: IDXGISwapChain3 =
        unsafe { factory.CreateSwapChain(command_queue, &swap_chain_desc) }
            .unwrap()
            .cast()
            .unwrap();
    let desc = unsafe { swap_chain.GetDesc().unwrap() };

    let renderer_heap: ID3D12DescriptorHeap = unsafe {
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

    let frames = (0..desc.BufferCount).map(|i| unsafe {
        let buf: ID3D12Resource = swap_chain.GetBuffer(i).unwrap();
        let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: rtv_start.ptr + (i * rtv_heap_inc_size) as usize,
        };
        dev.CreateRenderTargetView(buf.clone(), null(), rtv_handle);
        (buf, rtv_handle)
    }).collect::<Vec<_>>();

    let cpu_dh = unsafe { renderer_heap.GetCPUDescriptorHandleForHeapStart() };
    let gpu_dh = unsafe { renderer_heap.GetGPUDescriptorHandleForHeapStart() };

    let mut ctx = imgui::Context::create();
    let mut renderer = RenderEngine::new(
        &mut ctx,
        dev,
        2,
        DXGI_FORMAT_R8G8B8A8_UNORM,
        renderer_heap.clone(),
        cpu_dh,
        gpu_dh,
    );

    let diq: IDXGIInfoQueue = unsafe { DXGIGetDebugInterface1(0) }.unwrap();

    ctx.io_mut().display_size = [800., 600.];

    loop {
        trace!("Debug");
        unsafe {
            for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                let mut msg_len: usize = 0;
                diq.GetMessage(DXGI_DEBUG_ALL, i, null_mut(), &mut msg_len as _)
                    .unwrap();
                let diqm = vec![0u8; msg_len];
                let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                diq.GetMessage(DXGI_DEBUG_ALL, i, pdiqm, &mut msg_len as _)
                    .unwrap();
                let diqm = pdiqm.as_ref().unwrap();
                println!(
                    "{}",
                    String::from_utf8_lossy(std::slice::from_raw_parts(
                        diqm.pDescription as *const u8,
                        diqm.DescriptionByteLength
                    ))
                );
            }
            diq.ClearStoredMessages(DXGI_DEBUG_ALL);
        }

        trace!("New frame");
        renderer.new_frame(&mut ctx);

        let ui = ctx.frame();
        {
            let ui = &ui;
            Window::new("Hello world")
                .size([640.0, 480.0], Condition::Always)
                .build(&ui, || {
                    ui.text("Hello world!");
                    ui.text("こんにちは世界！");
                    ui.text("This...is...imgui-rs!");
                    ui.separator();
                    let mouse_pos = ui.io().mouse_pos;
                    ui.text(format!(
                        "Mouse Position: ({:.1},{:.1})",
                        mouse_pos[0], mouse_pos[1]
                    ));

                    imgui::ListBox::new("##listbox")
                        .size([300., 150.])
                        .build(ui, || {
                            imgui::Selectable::new("test1").build(ui);
                            imgui::Selectable::new("test2").build(ui);
                            imgui::Selectable::new("test3").selected(true).build(ui);
                            imgui::Selectable::new("test4").build(ui);
                            imgui::Selectable::new("test5").build(ui);
                        });

                    imgui::ComboBox::new("##combo")
                        .preview_value("test")
                        .build(ui, || {
                            imgui::Selectable::new("test1").build(ui);
                            imgui::Selectable::new("test2").build(ui);
                            imgui::Selectable::new("test3").selected(true).build(ui);
                            imgui::Selectable::new("test4").build(ui);
                            imgui::Selectable::new("test5").build(ui);
                        });
                    ui.open_popup("##combo");
                });
        };
        trace!("Render draw data");
        renderer.render_draw_data(ui.render(), &command_list, unsafe {
            swap_chain.GetCurrentBackBufferIndex()
        } as _);
        trace!("Present");
        unsafe { swap_chain.Present(1, 0) }.unwrap();

        trace!("Handle message");
        if !handle_message(hwnd) {
            break;
        }
    }
}

//
// Winapi things
//

fn handle_message(window: HWND) -> bool {
    unsafe {
        let mut msg = MaybeUninit::uninit();
        if GetMessageA(msg.as_mut_ptr(), window, 0, 0).as_bool() {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr()
                .as_ref()
                .map(|m| m.message != WM_QUIT)
                .unwrap_or(true)
        } else {
            false
        }
    }
}

pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut paint_struct = MaybeUninit::uninit();
            let mut rect = MaybeUninit::uninit();
            let hdc = BeginPaint(hwnd, paint_struct.as_mut_ptr());
            GetClientRect(hwnd, rect.as_mut_ptr());
            DrawTextA(
                hdc,
                "Test\0".as_bytes(),
                rect.as_mut_ptr(),
                DT_SINGLELINE | DT_CENTER | DT_VCENTER,
            );
            EndPaint(hwnd, paint_struct.as_mut_ptr());
        }
        WM_DESTROY => {
            PostQuitMessage(0);
        }
        _ => {
            return DefWindowProcA(hwnd, msg, w_param, l_param);
        }
    }
    LRESULT(0)
}

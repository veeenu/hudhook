// Based on https://github.com/Veykril/imgui-dx9-renderer
//
// Copyright (c) 2019 Lukas Wirth
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

#![no_main]

use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};

use hudhook::renderers::imgui_dx9::Renderer;
use imgui::Condition;
use tracing::metadata::LevelFilter;
use windows::core::PCSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3D9, IDirect3DDevice9, D3DADAPTER_DEFAULT,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_HAL, D3DFMT_R5G6B5, D3DMULTISAMPLE_NONE,
    D3DPRESENT_INTERVAL_DEFAULT, D3DPRESENT_PARAMETERS, D3DPRESENT_RATE_DEFAULT,
    D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::SystemServices::D3DCLEAR_TARGET;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
    PostQuitMessage, RegisterClassA, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, HCURSOR,
    HICON, HMENU, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 760.0;

unsafe fn setup_dx_context(hwnd: HWND) -> (IDirect3D9, IDirect3DDevice9) {
    let d9_option = Direct3DCreate9(D3D_SDK_VERSION);
    match d9_option {
        Some(d9) => {
            let mut present_params = D3DPRESENT_PARAMETERS {
                BackBufferWidth: WINDOW_WIDTH as _,
                BackBufferHeight: WINDOW_HEIGHT as _,
                BackBufferFormat: D3DFMT_R5G6B5,
                BackBufferCount: 1,
                MultiSampleType: D3DMULTISAMPLE_NONE,
                MultiSampleQuality: 0,
                SwapEffect: D3DSWAPEFFECT_DISCARD,
                hDeviceWindow: hwnd,
                Windowed: BOOL(1),
                EnableAutoDepthStencil: BOOL(0),
                Flags: 0,
                FullScreen_RefreshRateInHz: D3DPRESENT_RATE_DEFAULT,
                PresentationInterval: D3DPRESENT_INTERVAL_DEFAULT as u32,
                ..core::mem::zeroed()
            };
            let mut device: Option<IDirect3DDevice9> = None;
            match d9.CreateDevice(
                D3DADAPTER_DEFAULT,
                D3DDEVTYPE_HAL,
                hwnd,
                D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
                &mut present_params,
                &mut device,
            ) {
                Ok(_) => (d9, device.unwrap()),
                _ => panic!("CreateDevice failed"),
            }
        },
        None => panic!("Direct3DCreate9 failed"),
    }
}

#[no_mangle]
pub fn main(_argc: i32, _argv: *const *const u8) {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .init();

    let hinstance = unsafe { GetModuleHandleA(None).unwrap() };
    let wnd_class = WNDCLASSA {
        style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: HICON(0),
        hCursor: HCURSOR(0),
        hbrBackground: HBRUSH(0),
        lpszMenuName: PCSTR(null_mut()),
        lpszClassName: PCSTR("MyClass\0".as_ptr()),
    };
    unsafe { RegisterClassA(&wnd_class) };
    let mut rect = RECT { left: 0, top: 0, right: WINDOW_WIDTH as _, bottom: WINDOW_HEIGHT as _ };
    unsafe { AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, BOOL::from(false)) };
    let handle = unsafe {
        CreateWindowExA(
            WINDOW_EX_STYLE(0),
            PCSTR("MyClass\0".as_ptr()),
            PCSTR("MiniWIN\0".as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            // size and position
            100,
            100,
            rect.right - rect.left,
            rect.bottom - rect.top,
            HWND(0),
            HMENU(0),
            hinstance,
            null(),
        )
    };

    let (_, device) = unsafe { setup_dx_context(handle) };

    let mut ctx = imgui::Context::create();
    ctx.set_ini_filename(None);
    ctx.io_mut().display_size = [800., 600.];

    eprintln!("Creating renderer");
    let mut renderer = unsafe { Renderer::new(&mut ctx, device.clone()).unwrap() };

    let diq: IDXGIInfoQueue = unsafe { DXGIGetDebugInterface1(0) }.unwrap();

    loop {
        unsafe {
            for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                eprintln!("Debug Message {i}");
                let mut msg_len: usize = 0;
                diq.GetMessage(DXGI_DEBUG_ALL, i, null_mut(), &mut msg_len as _).unwrap();
                let diqm = vec![0u8; msg_len];
                let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                diq.GetMessage(DXGI_DEBUG_ALL, i, pdiqm, &mut msg_len as _).unwrap();
                let diqm = pdiqm.as_ref().unwrap();
                eprintln!(
                    "{}",
                    String::from_utf8_lossy(std::slice::from_raw_parts(
                        diqm.pDescription as *const u8,
                        diqm.DescriptionByteLength
                    ))
                );
            }
            diq.ClearStoredMessages(DXGI_DEBUG_ALL);
        }

        let ui = ctx.frame();
        ui.window("Hello world").size([640.0, 480.0], Condition::Always).build(|| {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));

            imgui::ListBox::new("##listbox").size([300., 150.]).build(ui, || {
                ui.selectable("test1");
                ui.selectable("test2");
                ui.selectable_config("test3").selected(true).build();
                ui.selectable("test4");
                ui.selectable("test5");
            });

            if ui.begin_combo("##combo", "test").is_some() {
                ui.selectable("test1");
                ui.selectable("test2");
                ui.selectable_config("test3").selected(true).build();
                ui.selectable("test4");
                ui.selectable("test5");
            };
            ui.open_popup("##combo");
        });
        unsafe {
            device.Clear(0, null_mut(), D3DCLEAR_TARGET as u32, 0xFFAA_AAAA, 1.0, 0).unwrap();
            device.BeginScene().unwrap();
        }

        renderer.render(ctx.render()).unwrap();
        unsafe {
            device.EndScene().unwrap();
            device.Present(null_mut(), null_mut(), None, null_mut()).unwrap();
        };

        if !handle_message(handle) {
            break;
        }
    }
}

fn handle_message(window: HWND) -> bool {
    unsafe {
        let mut msg = MaybeUninit::uninit();
        if GetMessageA(msg.as_mut_ptr(), window, 0, 0).0 > 0 {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr().as_ref().map(|m| m.message != WM_QUIT).unwrap_or(true)
        } else {
            false
        }
    }
}

#[allow(clippy::missing_safety_doc)]
pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
        },
        _ => {
            return DefWindowProcA(hwnd, msg, wparam, lparam);
        },
    }
    LRESULT(0)
}

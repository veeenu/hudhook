// Mandatory reference:
// https://www.codeslow.com/2019/12/tiny-windows-executable-in-rust.html

#![no_main]

use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};

use hudhook::renderers::imgui_dx11::RenderEngine;
use imgui::{Condition, Window};
use log::LevelFilter;
use simplelog::*;
use windows::core::PCSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DrawTextA, EndPaint, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HBRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExA, DefWindowProcA, DispatchMessageA, GetClientRect, GetMessageA, PostQuitMessage,
    RegisterClassA, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, HCURSOR, HICON, HMENU,
    WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

#[no_mangle]
pub fn main(_argc: i32, _argv: *const *const u8) {
    TermLogger::init(LevelFilter::Trace, Config::default(), TerminalMode::Mixed, ColorChoice::Auto)
        .unwrap();

    let hinstance = unsafe { GetModuleHandleA(None).unwrap() };
    let wnd_class = WNDCLASSA {
        style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: PCSTR("MyClass\0".as_ptr()),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hIcon: HICON(0),
        hCursor: HCURSOR(0),
        hbrBackground: HBRUSH(0),
        lpszMenuName: PCSTR(null_mut()),
    };
    unsafe { RegisterClassA(&wnd_class) };
    let handle = unsafe {
        CreateWindowExA(
            WINDOW_EX_STYLE(0),               // dwExStyle
            PCSTR("MyClass\0".as_ptr()),      // class we registered.
            PCSTR("MiniWIN\0".as_ptr()),      // title
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
            // size and position
            100,
            100,
            800,
            600,
            HWND(0),   // hWndParent
            HMENU(0),  // hMenu
            hinstance, // hInstance
            null(),
        )
    }; // lpParam

    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);
    imgui.io_mut().display_size = [800., 600.];

    eprintln!("Creating renderer");
    let mut renderer = RenderEngine::new(handle, &mut imgui);

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

        let ui = imgui.frame();
        Window::new("Hello world").size([640.0, 480.0], Condition::Always).build(&ui, || {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));

            imgui::ListBox::new("##listbox").size([300., 150.]).build(&ui, || {
                imgui::Selectable::new("test1").build(&ui);
                imgui::Selectable::new("test2").build(&ui);
                imgui::Selectable::new("test3").selected(true).build(&ui);
                imgui::Selectable::new("test4").build(&ui);
                imgui::Selectable::new("test5").build(&ui);
            });

            imgui::ComboBox::new("##combo").preview_value("test").build(&ui, || {
                imgui::Selectable::new("test1").build(&ui);
                imgui::Selectable::new("test2").build(&ui);
                imgui::Selectable::new("test3").selected(true).build(&ui);
                imgui::Selectable::new("test4").build(&ui);
                imgui::Selectable::new("test5").build(&ui);
            });
            ui.open_popup("##combo");
        });

        renderer.render_draw_data(ui.render()).unwrap();

        eprintln!("Present...");
        renderer.present();

        eprintln!("Handle message");
        if !handle_message(handle) {
            break;
        }
    }
}

// Winapi things

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

pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
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
        },
        WM_DESTROY => {
            PostQuitMessage(0);
        },
        _ => {
            return DefWindowProcA(hwnd, msg, wparam, lparam);
        },
    }
    LRESULT(0)
}

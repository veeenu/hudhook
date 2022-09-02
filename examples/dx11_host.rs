// Mandatory reference:
// https://www.codeslow.com/2019/12/tiny-windows-executable-in-rust.html

#![no_main]

use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DrawTextA, EndPaint, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HBRUSH,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, LoadLibraryExW, LOAD_LIBRARY_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExA, DefWindowProcA, DispatchMessageA, GetClientRect, GetMessageA, PostQuitMessage,
    RegisterClassA, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, HCURSOR, HICON, HMENU,
    WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

#[no_mangle]
pub fn main(_argc: i32, _argv: *const *const u8) {
    let hinstance = unsafe { GetModuleHandleA(None) }.unwrap();
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
        lpszMenuName: PCSTR(null()),
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
            640,
            480,
            HWND(0),   // hWndParent
            HMENU(0),  // hMenu
            hinstance, // hInstance
            null(),
        )
    }; // lpParam

    let diq: IDXGIInfoQueue = unsafe { DXGIGetDebugInterface1(0) }.unwrap();

    let mut dll_path = std::env::current_exe().unwrap();
    dll_path.pop();
    dll_path.push("hook_you.dll");

    println!("{:?}", dll_path.canonicalize());
    unsafe {
        LoadLibraryExW(
            PCWSTR(
                widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str())
                    .unwrap()
                    .as_ptr(),
            ),
            None,
            LOAD_LIBRARY_FLAGS(0),
        )
        .unwrap()
    };
    println!("Loaded library");

    loop {
        unsafe {
            for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                let mut msg_len: usize = 0;
                diq.GetMessage(DXGI_DEBUG_ALL, i, null_mut(), &mut msg_len as _).unwrap();
                let diqm = vec![0u8; msg_len];
                let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
                diq.GetMessage(DXGI_DEBUG_ALL, i, pdiqm, &mut msg_len as _).unwrap();
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

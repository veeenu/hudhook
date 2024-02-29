use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use windows::core::PCSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, D3DADAPTER_DEFAULT, D3DCLEAR_TARGET, D3DCREATE_SOFTWARE_VERTEXPROCESSING,
    D3DDEVTYPE_HAL, D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, PeekMessageA,
    PostQuitMessage, RegisterClassA, SetTimer, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    HCURSOR, HICON, HMENU, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSA,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

pub struct Dx9Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
    _caption: Arc<CString>,
}

impl Dx9Harness {
    #[allow(unused)]
    pub fn new(caption: &str) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let caption = Arc::new(CString::new(caption).unwrap());
        let child = Some(thread::spawn({
            let done = Arc::clone(&done);
            let caption = Arc::clone(&caption);

            move || {
                let hinstance = unsafe { GetModuleHandleA(None).unwrap() };
                let wnd_class = WNDCLASSA {
                    style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: PCSTR("MyClass\0".as_ptr()),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hIcon: HICON(0),
                    hCursor: HCURSOR(0),
                    hbrBackground: HBRUSH(0),
                    lpszMenuName: PCSTR(null_mut()),
                };
                unsafe { RegisterClassA(&wnd_class) };
                let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
                unsafe {
                    AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, BOOL::from(false))
                };
                let handle = unsafe {
                    CreateWindowExA(
                        WINDOW_EX_STYLE(0),
                        PCSTR("MyClass\0".as_ptr()),
                        PCSTR(caption.as_ptr().cast()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        // size and position
                        100,
                        100,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        HWND(0),
                        HMENU(0),
                        hinstance,
                        None,
                    )
                };

                let direct3d = unsafe { Direct3DCreate9(D3D_SDK_VERSION).unwrap() };
                let mut device = None;
                unsafe {
                    direct3d.CreateDevice(
                        D3DADAPTER_DEFAULT,
                        D3DDEVTYPE_HAL,
                        handle,
                        D3DCREATE_SOFTWARE_VERTEXPROCESSING as _,
                        &mut D3DPRESENT_PARAMETERS {
                            Windowed: BOOL::from(true),
                            SwapEffect: D3DSWAPEFFECT_DISCARD,
                            ..Default::default()
                        },
                        &mut device,
                    )
                };
                let device = device.unwrap();

                unsafe { SetTimer(handle, 0, 100, None) };

                loop {
                    eprintln!("Present...");
                    unsafe {
                        device.Clear(0, null(), D3DCLEAR_TARGET as _, 0x0022cc22, 1.0, 0);
                        device.Present(null(), null(), None, null());
                    }

                    eprintln!("Handle message");
                    if !handle_message(handle) {
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

impl Drop for Dx9Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
    }
}

#[allow(unused)]
fn handle_message(window: HWND) -> bool {
    unsafe {
        let mut msg = MaybeUninit::uninit();
        if PeekMessageA(msg.as_mut_ptr(), window, 0, 0, PM_REMOVE).0 > 0 {
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

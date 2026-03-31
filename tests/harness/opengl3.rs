use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use windows::core::PCSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, HBRUSH};
use windows::Win32::Graphics::OpenGL::{
    glClear, glClearColor, wglCreateContext, wglMakeCurrent, ChoosePixelFormat, SetPixelFormat,
    SwapBuffers, GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW,
    PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, PeekMessageA,
    PostQuitMessage, RegisterClassA, SetTimer, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    HCURSOR, HICON, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSA, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

pub struct Opengl3Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
    _caption: Arc<CString>,
}

impl Opengl3Harness {
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
                    lpszClassName: PCSTR(c"MyClass".as_ptr().cast()),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hIcon: HICON::default(),
                    hCursor: HCURSOR::default(),
                    hbrBackground: HBRUSH::default(),
                    lpszMenuName: PCSTR(null()),
                };
                unsafe { RegisterClassA(&wnd_class) };
                let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
                unsafe { AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, false) }
                    .unwrap();
                let hwnd = unsafe {
                    CreateWindowExA(
                        WINDOW_EX_STYLE::default(),
                        PCSTR(c"MyClass".as_ptr().cast()),
                        PCSTR(caption.as_ptr().cast()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
                        // size and position
                        100,
                        100,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        None,                   // hWndParent
                        None,                   // hMenu
                        Some(hinstance.into()), // hInstance
                        None,
                    )
                }
                .unwrap(); // lpParam

                let pfd = PIXELFORMATDESCRIPTOR {
                    nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                    nVersion: 1,
                    dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                    iPixelType: PFD_TYPE_RGBA,
                    cColorBits: 32,
                    cRedBits: 0,
                    cRedShift: 0,
                    cGreenBits: 0,
                    cGreenShift: 0,
                    cBlueBits: 0,
                    cBlueShift: 0,
                    cAlphaBits: 0,
                    cAlphaShift: 0,
                    cAccumBits: 0,
                    cAccumRedBits: 0,
                    cAccumGreenBits: 0,
                    cAccumBlueBits: 0,
                    cAccumAlphaBits: 0,
                    cDepthBits: 24,
                    cStencilBits: 8,
                    cAuxBuffers: 0,
                    iLayerType: PFD_MAIN_PLANE.0 as u8,
                    bReserved: 0,
                    dwLayerMask: 0,
                    dwVisibleMask: 0,
                    dwDamageMask: 0,
                };

                let window_handle = unsafe { GetDC(Some(hwnd)) };
                let pixel_format = unsafe { ChoosePixelFormat(window_handle, &pfd) };
                unsafe { SetPixelFormat(window_handle, pixel_format, &pfd).unwrap() };

                let context = unsafe { wglCreateContext(window_handle).unwrap() };
                unsafe { wglMakeCurrent(window_handle, context).unwrap() };

                unsafe { SetTimer(Some(hwnd), 0, 100, None) };

                loop {
                    unsafe {
                        glClearColor(0.2, 0.2, 0.2, 1.0);
                        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
                        SwapBuffers(window_handle);
                    }

                    if done.load(Ordering::SeqCst) {
                        break;
                    }

                    if !handle_message(hwnd) {
                        break;
                    }
                }
            }
        }));

        Self { child, done, _caption: caption }
    }
}

impl Drop for Opengl3Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
    }
}

fn handle_message(window: HWND) -> bool {
    let mut msg = MaybeUninit::uninit();
    unsafe {
        if PeekMessageA(msg.as_mut_ptr(), Some(window), 0, 0, PM_REMOVE).as_bool() {
            let msg = msg.assume_init();
            let _ = TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }
    }

    true
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => DefWindowProcA(hwnd, msg, wparam, lparam),
    }
}

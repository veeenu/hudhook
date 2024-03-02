use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use tracing::trace;
use windows::core::{s, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
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
    HCURSOR, HICON, HMENU, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSA,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
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

                let window_handle = unsafe { GetDC(hwnd) };

                let pixel_format = unsafe { ChoosePixelFormat(window_handle, &pfd) };
                unsafe { SetPixelFormat(window_handle, pixel_format, &pfd) };

                let con = unsafe { wglCreateContext(window_handle) }.unwrap();
                unsafe { wglMakeCurrent(window_handle, con) };

                unsafe { SetTimer(hwnd, 0, 100, None) };

                loop {
                    trace!("Debug");

                    unsafe { glClearColor(0.0, 1.0, 1.0, 1.0) }
                    unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT) };

                    unsafe { SwapBuffers(window_handle) };

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

impl Drop for Opengl3Harness {
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
        _ => {
            return DefWindowProcA(hwnd, msg, w_param, l_param);
        },
    }
    LRESULT(0)
}

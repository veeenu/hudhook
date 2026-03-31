use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};

use hudhook::util;
use windows::core::PCSTR;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, PeekMessageA,
    PostQuitMessage, RegisterClassA, SetTimer, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    HCURSOR, HICON, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY, WM_SIZE, WNDCLASSA,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

static RESIZE: OnceLock<Sender<(u32, u32)>> = OnceLock::new();

pub struct Dx11Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
    _caption: Arc<CString>,
}

impl Dx11Harness {
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
                    lpszClassName: PCSTR(c"MyClass".as_ptr().cast()),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hIcon: HICON::default(),
                    hCursor: HCURSOR::default(),
                    hbrBackground: HBRUSH::default(),
                    lpszMenuName: PCSTR(null_mut()),
                };
                unsafe { RegisterClassA(&wnd_class) };
                let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
                unsafe { AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, false) }
                    .unwrap();
                let hwnd = unsafe {
                    CreateWindowExA(
                        WINDOW_EX_STYLE(0),
                        PCSTR(c"MyClass".as_ptr().cast()),
                        PCSTR(caption.as_ptr().cast()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        // size and position
                        100,
                        100,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        None,
                        None,
                        Some(hinstance.into()),
                        None,
                    )
                }
                .unwrap();

                unsafe { util::enable_debug_interface() };

                let mut p_device: Option<ID3D11Device> = None;
                let mut p_swap_chain: Option<IDXGISwapChain> = None;
                let mut p_context: Option<ID3D11DeviceContext> = None;
                unsafe {
                    D3D11CreateDeviceAndSwapChain(
                        None,
                        D3D_DRIVER_TYPE_HARDWARE,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_FLAG(0),
                        Some(&[D3D_FEATURE_LEVEL_11_0]),
                        D3D11_SDK_VERSION,
                        Some(&DXGI_SWAP_CHAIN_DESC {
                            BufferDesc: DXGI_MODE_DESC {
                                Width: 800,
                                Height: 600,
                                RefreshRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                                ..Default::default()
                            },
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                            BufferCount: 1,
                            OutputWindow: hwnd,
                            Windowed: true.into(),
                            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                            ..Default::default()
                        }),
                        Some(&mut p_swap_chain),
                        Some(&mut p_device),
                        None,
                        Some(&mut p_context),
                    )
                    .unwrap();
                };

                let swap_chain = p_swap_chain.unwrap();

                unsafe {
                    SetTimer(Some(hwnd), 0, 100, None);
                }

                loop {
                    if done.load(Ordering::SeqCst) {
                        break;
                    }

                    unsafe {
                        swap_chain
                            .Present(1, windows::Win32::Graphics::Dxgi::DXGI_PRESENT(0))
                            .unwrap();
                    };

                    if !handle_message(hwnd) {
                        break;
                    }
                }
            }
        }));

        Self { child, done, _caption: caption }
    }
}

impl Drop for Dx11Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
    }
}

fn handle_message(window: HWND) -> bool {
    let mut msg = MaybeUninit::uninit();
    unsafe {
        if PeekMessageA(msg.as_mut_ptr(), Some(window), 0, 0, PM_REMOVE).0 > 0 {
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
        WM_SIZE => {
            if let Some(tx) = RESIZE.get() {
                tx.send(((lparam.0 & 0xFFFF) as u32, (lparam.0 >> 16) as u32)).unwrap();
            }
            LRESULT(0)
        },
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => DefWindowProcA(hwnd, msg, wparam, lparam),
    }
}

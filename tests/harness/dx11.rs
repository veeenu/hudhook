use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use log::LevelFilter;
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};
use windows::core::PCSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_DRIVER_TYPE_UNKNOWN};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11Device, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION, ID3D11DeviceContext, D3D11_CREATE_DEVICE_DEBUG, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory, DXGIGetDebugInterface1, IDXGIFactory, IDXGIInfoQueue, IDXGISwapChain,
    DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE, DXGI_SWAP_CHAIN_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT, DXGI_SWAP_EFFECT_DISCARD, DXGI_SWAP_EFFECT_FLIP_DISCARD,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
    PostQuitMessage, RegisterClassA, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, HCURSOR,
    HICON, HMENU, WINDOW_EX_STYLE, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WM_DESTROY,
};

pub struct Dx11Harness {
    child: Option<JoinHandle<()>>,
    done: Arc<AtomicBool>,
    _caption: Arc<CString>,
}

impl Dx11Harness {
    pub fn new(caption: &str) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let caption = Arc::new(CString::new(caption).unwrap());
        let child = Some(thread::spawn({
            let done = Arc::clone(&done);
            let caption = Arc::clone(&caption);

            move || {
                TermLogger::init(
                    LevelFilter::Trace,
                    Config::default(),
                    TerminalMode::Mixed,
                    ColorChoice::Auto,
                )
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
                let mut rect = RECT { left: 0, top: 0, right: 800, bottom: 600 };
                unsafe {
                    AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW | WS_VISIBLE, BOOL::from(false))
                };
                let handle = unsafe {
                    CreateWindowExA(
                        WINDOW_EX_STYLE(0),               // dwExStyle
                        PCSTR("MyClass\0".as_ptr()),      // class we registered.
                        PCSTR(caption.as_ptr().cast()),   // title
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE, // dwStyle
                        // size and position
                        100,
                        100,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        HWND(0),   // hWndParent
                        HMENU(0),  // hMenu
                        hinstance, // hInstance
                        null(),
                    )
                }; // lpParam

                let diq: IDXGIInfoQueue = unsafe { DXGIGetDebugInterface1(0) }.unwrap();

                let mut p_device: Option<ID3D11Device> = None;
                let mut p_swap_chain: Option<IDXGISwapChain> = None;
                let mut p_context: Option<ID3D11DeviceContext> = None;
                unsafe {
                    D3D11CreateDeviceAndSwapChain(
                        None,
                        D3D_DRIVER_TYPE_HARDWARE,
                        None,
                        D3D11_CREATE_DEVICE_FLAG(0),
                        &[D3D_FEATURE_LEVEL_11_0],
                        D3D11_SDK_VERSION,
                        &DXGI_SWAP_CHAIN_DESC {
                            BufferDesc: DXGI_MODE_DESC {
                                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                                ..Default::default()
                            },
                            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                            BufferCount: 2,
                            OutputWindow: handle,
                            Windowed: BOOL::from(true),
                            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                            ..Default::default()
                        },
                        &mut p_swap_chain,
                        &mut p_device,
                        null_mut(),
                        &mut p_context,
                    ).unwrap()
                };
                // let swap_chain = p_swap_chain.unwrap();

                loop {
                    unsafe {
                        for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
                            eprintln!("Debug Message {i}");
                            let mut msg_len: usize = 0;
                            diq.GetMessage(DXGI_DEBUG_ALL, i, null_mut(), &mut msg_len as _)
                                .unwrap();
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

                    eprintln!("Present...");
                    // unsafe { swap_chain.Present(1, 0).unwrap() };

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

impl Drop for Dx11Harness {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.child.take().unwrap().join().unwrap();
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

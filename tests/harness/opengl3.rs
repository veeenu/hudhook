use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use log::trace;
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, D3D12GetDebugInterface, ID3D12CommandAllocator, ID3D12CommandQueue,
    ID3D12Debug, ID3D12DescriptorHeap, ID3D12Device, ID3D12GraphicsCommandList, ID3D12Resource,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
    D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory, DXGIGetDebugInterface1, IDXGIFactory, IDXGIInfoQueue, IDXGISwapChain,
    IDXGISwapChain3, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{GetDC, HBRUSH};
use windows::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, SetPixelFormat, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE,
    PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExA, DefWindowProcA, DispatchMessageA, GetMessageA,
    PostQuitMessage, RegisterClassA, TranslateMessage, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, HCURSOR,
    HICON, HMENU, WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSA, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
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
                        null(),
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
                    iLayerType: PFD_MAIN_PLANE,
                    bReserved: 0,
                    dwLayerMask: 0,
                    dwVisibleMask: 0,
                    dwDamageMask: 0,
                };

                let window_handle = unsafe { GetDC(hwnd) };

                let pixel_format = unsafe { ChoosePixelFormat(window_handle, &pfd) };
                unsafe { SetPixelFormat(window_handle, pixel_format, &pfd) };

                loop {
                    trace!("Debug");
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
        if GetMessageA(msg.as_mut_ptr(), window, 0, 0).as_bool() {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr().as_ref().map(|m| m.message != WM_QUIT).unwrap_or(true)
        } else {
            false
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

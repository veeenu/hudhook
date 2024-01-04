use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;
use mh::{MH_ApplyQueued, MH_Initialize, MhHook, MH_STATUS};
use tracing::level_filters::LevelFilter;
use tracing::{error, info, trace};
use tracing_subscriber::prelude::*;
use windows::core::{w, Interface, HRESULT, PCSTR};
use windows::Win32::Foundation::{
    BOOL, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, MAX_PATH, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, D3DADAPTER_DEFAULT, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT,
    D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::RGNDATA;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExA, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DispatchMessageA, GetClientRect,
    GetDesktopWindow, GetMessageA, RegisterClassExW, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    WM_QUIT, WNDCLASSEXW, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};

mod dcomp;
pub mod imgui_dx12;
mod mh;

use dcomp::Dcomp;

////////////////////////////////////////////////////////////////////////////////
// Utils
////////////////////////////////////////////////////////////////////////////////

pub fn try_out_param<T, F, E, O>(mut f: F) -> Result<T, E>
where
    T: Default,
    F: FnMut(&mut T) -> Result<O, E>,
{
    let mut t: T = Default::default();
    match f(&mut t) {
        Ok(_) => Ok(t),
        Err(e) => Err(e),
    }
}

pub fn try_out_ptr<T, F, E, O>(mut f: F) -> Result<T, E>
where
    F: FnMut(&mut Option<T>) -> Result<O, E>,
{
    let mut t: Option<T> = None;
    match f(&mut t) {
        Ok(_) => Ok(t.unwrap()),
        Err(e) => Err(e),
    }
}

////////////////////////////////////////////////////////////////////////////////
// Create overlay window
////////////////////////////////////////////////////////////////////////////////
fn create_overlay_window(base_hwnd: HWND) -> HWND {
    let (width, height) = win_size(base_hwnd);

    unsafe {
        let wndclassex = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            lpszMenuName: w!("OverlayClass"),
            lpszClassName: w!("OverlayClass"),

            ..Default::default()
        };

        RegisterClassExW(&wndclassex);

        CreateWindowExW(
            WS_EX_TRANSPARENT,
            w!("OverlayClass"),
            w!("OverlayClass"),
            WS_VISIBLE | WS_POPUP,
            0,
            0,
            width,
            height,
            None,
            None,
            None,
            None,
        )
    }
}

fn win_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect).unwrap() };
    (rect.right - rect.left, rect.bottom - rect.top)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    trace!("msg {msg:?} wparam {wparam:?} lparam {lparam:?}");
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

////////////////////////////////////////////////////////////////////////////////
// Hooking logic
////////////////////////////////////////////////////////////////////////////////
type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain, SyncInterval: u32, Flags: u32) -> HRESULT;

type Dx9PresentFn = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut RENDERER: OnceLock<Mutex<Dcomp>> = OnceLock::new();
static mut TRAMPOLINE_DX11: OnceLock<DXGISwapChainPresentType> = OnceLock::new();
static mut TRAMPOLINE_DX9: OnceLock<Dx9PresentFn> = OnceLock::new();

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let trampoline =
        TRAMPOLINE_DX11.get().expect("IDXGISwapChain::Present trampoline uninitialized");
    GAME_HWND.get_or_init(|| {
        let mut desc = Default::default();
        p_this.GetDesc(&mut desc).unwrap();
        info!("Output window: {:?}", p_this);
        info!("Desc: {:?}", desc);
        desc.OutputWindow
    });

    if let Some(renderer) = RENDERER.get() {
        if let Ok(mut renderer) = renderer.try_lock() {
            if let Err(e) = renderer.render() {
                error!("Render: {e:?}");
            }
        }
    }

    trace!("Call trampoline");
    trampoline(p_this, sync_interval, flags)
}

fn get_present_addr() -> DXGISwapChainPresentType {
    let mut p_device: Option<ID3D11Device> = None;
    let mut p_context: Option<ID3D11DeviceContext> = None;
    let mut p_swap_chain: Option<IDXGISwapChain> = None;

    let dummy_hwnd = unsafe { GetDesktopWindow() };
    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_NULL,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&[D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                    Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
                    ..Default::default()
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 1,
                OutputWindow: dummy_hwnd,
                Windowed: BOOL(1),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, ..Default::default() },
                ..Default::default()
            }),
            Some(&mut p_swap_chain),
            Some(&mut p_device),
            None,
            Some(&mut p_context),
        )
        .expect("D3D11CreateDeviceAndSwapChain failed");
    }

    let swap_chain = p_swap_chain.unwrap();

    let present_ptr = swap_chain.vtable().Present;

    unsafe { std::mem::transmute(present_ptr) }
}

unsafe extern "system" fn imgui_dx9_present_impl(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    trace!("IDirect3DDevice9::Present invoked");

    GAME_HWND.get_or_init(|| {
        let mut dcp = Default::default();
        this.GetCreationParameters(&mut dcp).unwrap();
        dcp.hFocusWindow
    });

    if let Some(renderer) = RENDERER.get() {
        if let Ok(mut renderer) = renderer.try_lock() {
            if let Err(e) = renderer.render() {
                error!("Render: {e:?}");
            }
        }
    }

    let trampoline_present =
        TRAMPOLINE_DX9.get().expect("IDirect3DDevice9::Present trampoline uninitialized");

    trampoline_present(this, psourcerect, pdestrect, hdestwindowoverride, pdirtyregion)
}

unsafe fn get_dx9_present_addr() -> Dx9PresentFn {
    let d9 = Direct3DCreate9(D3D_SDK_VERSION).unwrap();

    let mut d3d_display_mode =
        D3DDISPLAYMODE { Width: 0, Height: 0, RefreshRate: 0, Format: D3DFORMAT(0) };
    d9.GetAdapterDisplayMode(D3DADAPTER_DEFAULT, &mut d3d_display_mode).unwrap();

    let mut present_params = D3DPRESENT_PARAMETERS {
        Windowed: BOOL(1),
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        BackBufferFormat: d3d_display_mode.Format,
        ..core::mem::zeroed()
    };

    let dummy_hwnd = unsafe { GetDesktopWindow() };
    let device: IDirect3DDevice9 = try_out_ptr(|v| {
        d9.CreateDevice(
            D3DADAPTER_DEFAULT,
            D3DDEVTYPE_NULLREF,
            dummy_hwnd, // GetDesktopWindow(),
            D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
            &mut present_params,
            v,
        )
    })
    .expect("IDirect3DDevice9::CreateDevice: failed to create device");

    let present_ptr = device.vtable().Present;

    std::mem::transmute(present_ptr)
}

unsafe fn hook_dx11() {
    match unsafe { MH_Initialize() } {
        MH_STATUS::MH_ERROR_ALREADY_INITIALIZED | MH_STATUS::MH_OK => {},
        status @ MH_STATUS::MH_ERROR_MEMORY_ALLOC => panic!("MH_Initialize: {status:?}"),
        _ => unreachable!(),
    }

    let present_addr = get_present_addr();
    let hook_present =
        MhHook::new(present_addr as *mut _, dxgi_swap_chain_present_impl as _).unwrap();
    TRAMPOLINE_DX11.get_or_init(|| std::mem::transmute(hook_present.trampoline()));

    hook_present.queue_enable().unwrap();

    MH_ApplyQueued().ok_context("").unwrap();
}

unsafe fn hook_dx9() {
    match unsafe { MH_Initialize() } {
        MH_STATUS::MH_ERROR_ALREADY_INITIALIZED | MH_STATUS::MH_OK => {},
        status @ MH_STATUS::MH_ERROR_MEMORY_ALLOC => panic!("MH_Initialize: {status:?}"),
        _ => unreachable!(),
    }

    let present_addr = get_dx9_present_addr();
    let hook_present = MhHook::new(present_addr as *mut _, imgui_dx9_present_impl as _).unwrap();
    TRAMPOLINE_DX9.get_or_init(|| std::mem::transmute(hook_present.trampoline()));

    hook_present.queue_enable().unwrap();

    MH_ApplyQueued().ok_context("").unwrap();
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

unsafe fn run_dcomp() -> Result<()> {
    let base_hwnd = loop {
        match GAME_HWND.get().copied() {
            Some(hwnd) => break hwnd,
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    info!("Found hwnd {base_hwnd:?}");

    // let hwnd = create_overlay_window(base_hwnd);

    let renderer = Dcomp::new(base_hwnd)?;
    info!("Built dcomp");

    RENDERER.get_or_init(move || Mutex::new(renderer));

    // loop {
    // if let Err(e) = renderer.render() {
    //     error!("Render error: {e:?}");
    //     break;
    // }

    // if !handle_message(hwnd) {
    //     break;
    // }
    // }

    Ok(())
}

fn get_dll_path() -> Option<PathBuf> {
    let mut hmodule = HMODULE(0);
    if let Err(e) = unsafe {
        GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCSTR("DllMain".as_ptr() as _),
            &mut hmodule,
        )
    } {
        tracing::error!("get_dll_path: GetModuleHandleExA error: {e:?}");
        return None;
    }

    let mut sz_filename = [0u16; MAX_PATH as usize];
    // SAFETY
    // pointer to sz_filename always defined and MAX_PATH bounds manually checked
    let len = unsafe { GetModuleFileNameW(hmodule, &mut sz_filename) } as usize;

    Some(OsString::from_wide(&sz_filename[..len]).into())
}

#[no_mangle]
pub unsafe extern "stdcall" fn DllMain(
    hmodule: HINSTANCE,
    reason: u32,
    _: *mut ::std::ffi::c_void,
) {
    if reason == DLL_PROCESS_ATTACH {
        ::std::thread::spawn(move || {
            let log_file = get_dll_path()
                .map(|mut path| {
                    path.pop();
                    path.push("experiment.log");
                    path
                })
                .map(std::fs::File::create)
                .unwrap()
                .unwrap();

            let file_layer = tracing_subscriber::fmt::layer()
                .with_thread_ids(true)
                .with_file(true)
                .with_line_number(true)
                .with_thread_names(true)
                .with_writer(Mutex::new(log_file))
                .with_ansi(false)
                .boxed();

            tracing_subscriber::registry().with(LevelFilter::TRACE).with(file_layer).init();
            info!("Hooking...");
            hook_dx11();

            info!("Doing thing...");
            if let Err(e) = run_dcomp() {
                error!("Run dcomp: {e:?}");
            }
        });
    }
}

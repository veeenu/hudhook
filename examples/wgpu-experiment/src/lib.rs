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
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
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

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut RENDERER: OnceLock<Mutex<Dcomp>> = OnceLock::new();
static mut TRAMPOLINE: OnceLock<DXGISwapChainPresentType> = OnceLock::new();

unsafe extern "system" fn dxgi_swap_chain_present_impl(
    p_this: IDXGISwapChain,
    sync_interval: u32,
    flags: u32,
) -> HRESULT {
    let trampoline = TRAMPOLINE.get().expect("IDXGISwapChain::Present trampoline uninitialized");
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

unsafe fn hook() {
    match unsafe { MH_Initialize() } {
        MH_STATUS::MH_ERROR_ALREADY_INITIALIZED | MH_STATUS::MH_OK => {},
        status @ MH_STATUS::MH_ERROR_MEMORY_ALLOC => panic!("MH_Initialize: {status:?}"),
        _ => unreachable!(),
    }

    let present_addr = get_present_addr();
    let hook_present =
        MhHook::new(present_addr as *mut _, dxgi_swap_chain_present_impl as _).unwrap();
    TRAMPOLINE.get_or_init(|| std::mem::transmute(hook_present.trampoline()));

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

    // let hwnd = create_overlay_window(base_hwnd);

    let renderer = Dcomp::new(base_hwnd)?;

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
            hook();

            info!("Doing thing...");
            if let Err(e) = run_dcomp() {
                error!("Run dcomp: {e:?}");
            }
        });
    }
}

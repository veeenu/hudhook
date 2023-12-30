use std::ffi::{c_void, OsString};
use std::mem::MaybeUninit;
use std::os::windows::prelude::OsStringExt;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use imgui::*;
use imgui_wgpu::{Renderer, RendererConfig};
use mh::{MH_ApplyQueued, MH_Initialize, MhHook, MH_STATUS};
use pollster::block_on;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;
use wgpu::{Backends, Device, Instance, InstanceDescriptor, Queue, Surface, TextureFormat};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dwm::{
    DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED,
    DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    CreateRectRgn, DeleteObject, RedrawWindow, UpdateWindow, HBRUSH, HRGN, RDW_INTERNALPAINT,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExA, GetModuleHandleW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::*;

mod mh;

struct Raw {
    hwnd: HWND,
}

unsafe impl HasRawWindowHandle for Raw {
    fn raw_window_handle(&self) -> raw_window_handle::RawWindowHandle {
        let mut handle = Win32WindowHandle::empty();
        handle.hwnd = self.hwnd.0 as *mut c_void;
        handle.into()
    }
}

unsafe impl HasRawDisplayHandle for Raw {
    fn raw_display_handle(&self) -> raw_window_handle::RawDisplayHandle {
        WindowsDisplayHandle::empty().into()
    }
}

struct Wgpu {
    device: Device,
    surface: Surface,
    queue: Queue,
    context: Context,
    renderer: Renderer,
}

impl Wgpu {
    fn new(hwnd: HWND) -> Self {
        // Create an instance
        let instance =
            Instance::new(InstanceDescriptor { backends: Backends::DX12, ..Default::default() });

        let mut rect = Default::default();
        unsafe { GetWindowRect(hwnd, &mut rect).unwrap() };

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        let raw = Raw { hwnd };

        // Create a surface
        let surface = unsafe { instance.create_surface(&raw).unwrap() };

        // Request an adapter
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .unwrap();

        // Create a device and queue
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None)).unwrap();

        // Configure the surface
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: TextureFormat::Bgra8Unorm,
            width: width as _,
            height: height as _,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Bgra8Unorm],
        };
        surface.configure(&device, &config);

        let mut context = Context::create();
        context.set_ini_filename(None);

        let hidpi_factor = 2.0;
        let font_size = 13.0 * hidpi_factor;
        context.io_mut().font_global_scale = 1.0 / hidpi_factor;
        context.io_mut().display_size = [width as _, height as _];

        // Configure fonts
        context.fonts().add_font(&[FontSource::DefaultFontData {
            config: Some(FontConfig {
                oversample_h: 1,
                pixel_snap_h: true,
                size_pixels: font_size,
                ..Default::default()
            }),
        }]);

        // Create the imgui-wgpu renderer
        let renderer_config =
            RendererConfig { texture_format: config.format, ..Default::default() };
        let renderer = Renderer::new(&mut context, &device, &queue, renderer_config);

        Self { device, surface, queue, context, renderer }
    }
}

unsafe fn create_overlay_window(target_hwnd: HWND) -> Result<HWND> {
    let hinstance = GetModuleHandleW(None).unwrap().into();

    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as _,
        style: CS_HREDRAW | CS_VREDRAW,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hbrBackground: HBRUSH(0),
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: w!("YourOverlayClass"),
        ..Default::default()
    };

    let mut rect = RECT::default();
    GetWindowRect(target_hwnd, &mut rect).unwrap();

    // Register the window class
    RegisterClassExW(&window_class);

    // Create the window
    let hwnd = CreateWindowExW(
        WS_EX_ACCEPTFILES | WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_COMPOSITED,
        // WS_EX_TOPMOST | WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP,
        // WS_EX_TOPMOST | WS_EX_LAYERED,
        w!("YourOverlayClass"),
        w!("Overlay"),
        WS_VISIBLE
            | WS_CAPTION
            | WS_BORDER
            | WS_CLIPSIBLINGS
            | WS_CLIPCHILDREN
            | WS_THICKFRAME
            | WS_OVERLAPPED
            | WS_MINIMIZEBOX
            | WS_MAXIMIZEBOX
            | WS_SYSMENU,
        rect.left,
        rect.top,
        (rect.right - rect.left) / 2,
        (rect.bottom - rect.top) / 2,
        target_hwnd,
        None,
        GetModuleHandleW(None).unwrap(),
        None,
    );

    let region = CreateRectRgn(0, 0, -1, -1);

    let bb = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
        fEnable: true.into(),
        hRgnBlur: region,
        fTransitionOnMaximized: true.into(),
    };
    DwmEnableBlurBehindWindow(hwnd, &bb).unwrap();
    DeleteObject(region);

    Ok(hwnd)
}

type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Handle window messages
    // ...
    match msg {
        WM_PAINT => {
            if let Some(wgpu) = WGPU.get() {
                render(&mut wgpu.lock().unwrap());
            }
            // RedrawWindow(hwnd, None, HRGN(0), RDW_INTERNALPAINT);
        },
        WM_MOVE => {},
        WM_DESTROY => {
            PostQuitMessage(0);
        },
        _ => {},
    }

    // if let Some(game_hwnd) = GAME_HWND.get().copied() {
    //     let wnd_proc =
    //         std::mem::transmute::<_, WndProcType>(GetWindowLongPtrW(game_hwnd,
    // GWLP_WNDPROC));     wnd_proc(game_hwnd, msg, wparam, lparam);
    // }

    DefWindowProcW(hwnd, msg, wparam, lparam)
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

fn render(wgpu: &mut Wgpu) {
    let clear_color = wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 0.0 };

    let ui = wgpu.context.frame();

    ui.show_demo_window(&mut true);
    ui.end_frame_early();

    let [width, height] = wgpu.context.io_mut().display_size;

    let mut encoder: wgpu::CommandEncoder =
        wgpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

    let frame = match wgpu.surface.get_current_texture() {
        Ok(frame) => frame,
        Err(e) => {
            eprintln!("dropped frame: {e:?}");
            return;
        },
    };
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear_color), store: true },
        })],
        depth_stencil_attachment: None,
    });

    wgpu.renderer.render(wgpu.context.render(), &wgpu.queue, &wgpu.device, &mut rpass).unwrap();

    drop(rpass);
    wgpu.queue.submit(Some(encoder.finish()));

    frame.present();
}

pub fn run() {
    unsafe {
        let game_hwnd = loop {
            if let Some(game_hwnd) = GAME_HWND.get().copied() {
                break game_hwnd;
            } else {
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        info!("Got game hwnd {game_hwnd:?}");

        let hwnd = create_overlay_window(game_hwnd).unwrap();
        let mut wgpu = Wgpu::new(hwnd);
        info!("Made hwnd {hwnd:?}");
        render(&mut wgpu);

        WGPU.get_or_init(move || Mutex::new(wgpu));

        loop {
            RedrawWindow(hwnd, None, HRGN(0), RDW_INTERNALPAINT);
            UpdateWindow(hwnd);

            if !handle_message(hwnd) {
                break;
            }
        }
    }
}

type DXGISwapChainPresentType =
    unsafe extern "system" fn(This: IDXGISwapChain, SyncInterval: u32, Flags: u32) -> HRESULT;

static mut GAME_HWND: OnceLock<HWND> = OnceLock::new();
static mut WGPU: OnceLock<Mutex<Wgpu>> = OnceLock::new();
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
        tracing::info!("Output window: {:?}", p_this);
        desc.OutputWindow
    });

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
                OutputWindow: dummy_hwnd, // GetDesktopWindow(),
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
            run();
        });
    }
}

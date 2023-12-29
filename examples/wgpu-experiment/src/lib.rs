use imgui::*;
use imgui_wgpu::{Renderer, RendererConfig};
use pollster::block_on;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::{mem::MaybeUninit, time::Instant};
use wgpu::{Backends, Device, Instance, InstanceDescriptor, Queue, Surface, TextureFormat};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::{
            InvalidateRgn, RedrawWindow, HRGN, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME,
            RDW_INTERNALPAINT, RDW_INVALIDATE,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

mod mh;

const WIDTH: usize = 1920;
const HEIGHT: usize = 800;

unsafe fn create_overlay_window(target_hwnd: HWND) -> Result<(HWND, HINSTANCE)> {
    let hinstance = GetModuleHandleW(None).unwrap().into();
    // Define your window class
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: w!("YourOverlayClass"),
        ..Default::default()
    };

    // Register the window class
    RegisterClassW(&window_class);

    // Create the window
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST,
        w!("YourOverlayClass"),
        w!("Overlay"),
        WS_VISIBLE | WS_OVERLAPPEDWINDOW,
        0,
        0,
        WIDTH as _,
        HEIGHT as _,
        None,
        None,
        GetModuleHandleW(None).unwrap(),
        None,
    );
    // SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_COLORKEY).unwrap();

    Ok((hwnd, hinstance))
}

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
            InvalidateRgn(hwnd, None, BOOL::from(false));
            RedrawWindow(
                hwnd,
                None,
                HRGN(0),
                RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN,
            );
        },
        WM_DESTROY => {
            PostQuitMessage(0);
        },
        _ => {},
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

struct Rwh {
    window_handle: Win32WindowHandle,
}

unsafe impl HasRawWindowHandle for Rwh {
    fn raw_window_handle(&self) -> raw_window_handle::RawWindowHandle {
        self.window_handle.into()
    }
}

unsafe impl HasRawDisplayHandle for Rwh {
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
        unsafe { GetClientRect(hwnd, &mut rect).unwrap() };

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        let mut window_handle = Win32WindowHandle::empty();
        window_handle.hwnd = hwnd.0 as *mut _;
        window_handle.hinstance =
            unsafe { GetWindowLongW(hwnd, GWLP_HINSTANCE) } as *mut std::ffi::c_void;

        let rwh = Rwh { window_handle };

        // Create a surface
        let surface = unsafe { instance.create_surface(&rwh).unwrap() };

        // Request an adapter
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
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
            format: TextureFormat::Bgra8UnormSrgb,
            width: width as _,
            height: height as _,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb],
        };
        surface.configure(&device, &config);

        let mut context = Context::create();
        context.set_ini_filename(None);

        let hidpi_factor = 2.0;
        let font_size = 13.0 * hidpi_factor;
        context.io_mut().font_global_scale = 1.0 / hidpi_factor;
        context.io_mut().display_size = [width as _, height as _];
        // context.io_mut().display_framebuffer_scale = [1.0, 1.0];

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

pub fn do_thing() {
    unsafe {
        let (hwnd, _) = create_overlay_window(GetDesktopWindow()).unwrap();
        let mut wgpu = Wgpu::new(hwnd);

        let clear_color = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        let mut last_frame = Instant::now();

        loop {
            let now = Instant::now();
            wgpu.context.io_mut().update_delta_time(now - last_frame);
            last_frame = now;

            let ui = wgpu.context.frame();

            ui.show_demo_window(&mut true);

            ui.end_frame_early();

            let [width, height] = wgpu.context.io_mut().display_size;

            {
                let surface_desc = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    width: width as _,
                    height: height as _,
                    present_mode: wgpu::PresentMode::Fifo,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![wgpu::TextureFormat::Bgra8Unorm],
                };

                wgpu.surface.configure(&wgpu.device, &surface_desc);
            }

            let mut encoder: wgpu::CommandEncoder =
                wgpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            let frame = match wgpu.surface.get_current_texture() {
                Ok(frame) => frame,
                Err(e) => {
                    eprintln!("dropped frame: {e:?}");
                    continue;
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

            wgpu.renderer
                .render(wgpu.context.render(), &wgpu.queue, &wgpu.device, &mut rpass)
                .unwrap();

            drop(rpass);
            wgpu.queue.submit(Some(encoder.finish()));

            frame.present();

            if !handle_message(hwnd) {
                break;
            }
        }
    }
}

// #[no_mangle]
// pub unsafe extern "stdcall" fn DllMain(
//     hmodule: HINSTANCE,
//     reason: u32,
//     _: *mut ::std::ffi::c_void,
// ) {
//     if reason == DLL_PROCESS_ATTACH {
//         ::std::thread::spawn(move || {});
//     }
// }

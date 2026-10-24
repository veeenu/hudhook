mod harness;
#[allow(dead_code)]
mod hook;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use harness::dx11::Dx11Harness;
use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::{Hudhook, ImguiRenderLoop};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11DeviceContext, D3D11_VIEWPORT};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC};
use windows::Win32::Graphics::Dxgi::*;

#[derive(Default)]
struct Sizes {
    initializations: u32,
    frames: u32,
    before_render: [f32; 2],
    viewport: [f32; 2],
    context: Option<ID3D11DeviceContext>,
}

struct SizeProbe(Arc<Mutex<Sizes>>);

impl ImguiRenderLoop for SizeProbe {
    fn initialize(&mut self, _: &mut imgui::Context, _: &mut dyn hudhook::RenderContext) {
        self.0.lock().unwrap().initializations += 1;
    }

    fn before_render(&mut self, ctx: &mut imgui::Context, _: &mut dyn hudhook::RenderContext) {
        self.0.lock().unwrap().before_render = ctx.io().display_size;
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        let context = self.0.lock().unwrap().context.clone().unwrap();
        let sizes = Arc::clone(&self.0);
        let draw_list = ui.get_foreground_draw_list();
        // Keep the callback's draw list nonempty for imgui 0.12.
        draw_list.add_rect([0., 0.], [1., 1.], [1., 1., 1., 1.]).build();
        draw_list
            .add_callback(move || unsafe {
                let mut count = 1;
                let mut viewport = D3D11_VIEWPORT::default();
                context.RSGetViewports(&mut count, Some(&mut viewport));
                let mut sizes = sizes.lock().unwrap();
                sizes.frames += 1;
                sizes.viewport = [viewport.Width, viewport.Height];
            })
            .build();
    }
}

#[test]
fn test_imgui_dx11() {
    hook::setup_tracing();
    let sizes = Arc::new(Mutex::new(Sizes::default()));
    let completed = Arc::new(Mutex::new(false));
    let dx11_harness = Dx11Harness::with_frame_callback("DX11 hook example", true, {
        let sizes = Arc::clone(&sizes);
        let completed = Arc::clone(&completed);
        let mut step = 0;
        let mut frames = 0;
        move |swap_chain, context| unsafe {
            let mut snapshot = sizes.lock().unwrap();
            snapshot.context = Some(context.clone());
            if snapshot.frames == frames || *completed.lock().unwrap() {
                return;
            }
            frames = snapshot.frames;
            let (mut width, mut height) = (0, 0);
            let swap_chain2: IDXGISwapChain2 = swap_chain.cast().unwrap();
            swap_chain2.GetSourceSize(&mut width, &mut height).unwrap();
            assert_eq!(snapshot.before_render, [width as f32, height as f32]);
            assert_eq!(snapshot.viewport, snapshot.before_render);
            let mut targets = [None];
            context.OMGetRenderTargets(Some(&mut targets), None);
            assert!(targets[0].is_none(), "Overlay retained a back-buffer binding");
            drop(snapshot);
            let flags = DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH;
            match step {
                0 => swap_chain.ResizeBuffers(0, 1024, 768, DXGI_FORMAT_UNKNOWN, flags).unwrap(),
                1 => swap_chain2.SetSourceSize(640, 480).unwrap(),
                2 => {
                    // A stale WM_SIZE must not overwrite the actual source
                    // dimensions.
                    let hwnd = swap_chain.GetDesc().unwrap().OutputWindow;
                    windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::WM_SIZE,
                        None,
                        Some(windows::Win32::Foundation::LPARAM((96 << 16) | 128)),
                    );
                },
                3 => swap_chain.ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, flags).unwrap(),
                4 => {
                    let before = sizes.lock().unwrap().frames;
                    swap_chain
                        .cast::<IDXGISwapChain1>()
                        .unwrap()
                        .Present1(0, DXGI_PRESENT_TEST, &DXGI_PRESENT_PARAMETERS::default())
                        .ok()
                        .unwrap();
                    assert_eq!(sizes.lock().unwrap().frames, before);
                    swap_chain
                        .cast::<IDXGISwapChain1>()
                        .unwrap()
                        .Present1(0, DXGI_PRESENT(0), &DXGI_PRESENT_PARAMETERS::default())
                        .ok()
                        .unwrap();
                    assert_eq!(sizes.lock().unwrap().frames, before + 1);
                },
                5 => match swap_chain.SetFullscreenState(true, None) {
                    Ok(()) => {
                        swap_chain.ResizeBuffers(0, 800, 600, DXGI_FORMAT_UNKNOWN, flags).unwrap();
                    },
                    Err(error) if error.code() == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE => {},
                    Err(error) => panic!("Fullscreen transition failed: {error:?}"),
                },
                6 => {
                    swap_chain.SetFullscreenState(false, None).unwrap();
                    swap_chain
                        .ResizeTarget(&DXGI_MODE_DESC {
                            Width: 800,
                            Height: 600,
                            ..Default::default()
                        })
                        .unwrap();
                    swap_chain.ResizeBuffers(0, 800, 600, DXGI_FORMAT_UNKNOWN, flags).unwrap();
                },
                _ => {
                    *completed.lock().unwrap() = true;
                },
            }
            step += 1;
        }
    });
    let timeout = Instant::now();
    while sizes.lock().unwrap().context.is_none() {
        assert!(timeout.elapsed() < Duration::from_secs(5), "Harness did not start");
        thread::sleep(Duration::from_millis(10));
    }
    Hudhook::builder()
        .with::<ImguiDx11Hooks>(SizeProbe(Arc::clone(&sizes)))
        .build()
        .apply()
        .unwrap();
    let timeout = Instant::now();
    while !*completed.lock().unwrap() {
        assert!(timeout.elapsed() < Duration::from_secs(10), "Resize regression did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    drop(dx11_harness);
    let previous = sizes.lock().unwrap().initializations;
    let completed = Arc::new(Mutex::new(false));
    let replacement = Dx11Harness::with_frame_callback("DX11 replacement", false, {
        let sizes = Arc::clone(&sizes);
        let completed = Arc::clone(&completed);
        let mut resized = false;
        move |swap_chain, context| unsafe {
            let mut snapshot = sizes.lock().unwrap();
            snapshot.context = Some(context.clone());
            if snapshot.initializations <= previous {
                return;
            }
            assert_eq!(snapshot.before_render, if resized { [1024., 768.] } else { [800., 600.] });
            assert_eq!(snapshot.viewport, snapshot.before_render);
            let mut targets = [None];
            context.OMGetRenderTargets(Some(&mut targets), None);
            assert!(targets[0].is_none(), "Overlay retained a blt back-buffer binding");
            drop(snapshot);
            if resized {
                *completed.lock().unwrap() = true;
            } else {
                swap_chain
                    .ResizeBuffers(
                        0,
                        1024,
                        768,
                        DXGI_FORMAT_UNKNOWN,
                        DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH,
                    )
                    .unwrap();
                resized = true;
            }
        }
    });
    let timeout = Instant::now();
    while !*completed.lock().unwrap() {
        assert!(timeout.elapsed() < Duration::from_secs(10), "Device replacement did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    drop(replacement);
}

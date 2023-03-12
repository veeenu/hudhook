mod harness;

use std::thread;
use std::time::Duration;

use harness::dx11::Dx11Harness;
use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::hooks::{self, ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::{Condition, StyleColor};

#[test]
fn test_imgui_dx11() {
    struct Dx11HookExample;

    impl Dx11HookExample {
        fn new() -> Self {
            println!("Initializing");
            hudhook::utils::alloc_console();

            Dx11HookExample
        }
    }

    impl ImguiRenderLoop for Dx11HookExample {
        fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
            ui.window("Hello world").size([300.0, 300.0], Condition::FirstUseEver).build(|| {
                ui.text("Hello world!");
                ui.text("こんにちは世界！");
                ui.text("This...is...imgui-rs!");
                for y in 0..16 {
                    for x in 0..16 {
                        let btn = y * 16 + x;
                        let _token = ui.push_style_color(
                            StyleColor::Text,
                            if ui.io().keys_down[btn as usize] {
                                [0., 1., 0., 1.]
                            } else {
                                [1., 1., 1., 1.]
                            },
                        );
                        ui.text(format!("{btn:02x}"));
                        ui.same_line();
                    }
                    ui.new_line();
                }
                ui.text(if ui.io().key_shift { "SHIFT" } else { "shift" });
                ui.text(if ui.io().key_ctrl { "CTRL" } else { "ctrl" });
                ui.text(if ui.io().key_alt { "ALT" } else { "alt" });
                ui.separator();
                let mouse_pos = ui.io().mouse_pos;
                ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
            });
        }
    }

    let dx11_harness = Dx11Harness::new("DX11 hook example");
    thread::sleep(Duration::from_millis(500));

    unsafe {
        let hooks: Box<dyn hooks::Hooks> = { Dx11HookExample::new().into_hook::<ImguiDx11Hooks>() };
        hooks.hook();
        hudhook::lifecycle::global_state::set_hooks(hooks);
    }

    thread::sleep(Duration::from_millis(5000));
    drop(dx11_harness);
}

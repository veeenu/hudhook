mod harness;

use std::thread;
use std::time::Duration;

use harness::dx12::Dx12Harness;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::hooks::{self, ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::Condition;

#[test]
fn test_imgui_dx12() {
    struct Dx12HookExample;

    impl Dx12HookExample {
        fn new() -> Self {
            println!("Initializing");
            hudhook::utils::alloc_console();

            Dx12HookExample
        }
    }

    impl ImguiRenderLoop for Dx12HookExample {
        fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
            ui.window("Hello world").size([300.0, 300.0], Condition::FirstUseEver).build(|| {
                ui.text("Hello world!");
                ui.text("こんにちは世界！");
                ui.text("This...is...imgui-rs!");
                ui.separator();
                let mouse_pos = ui.io().mouse_pos;
                ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
            });
        }
    }

    let dx12_harness = Dx12Harness::new("DX12 hook example");
    thread::sleep(Duration::from_millis(500));

    unsafe {
        let hooks: Box<dyn hooks::Hooks> = { Dx12HookExample::new().into_hook::<ImguiDx12Hooks>() };
        hooks.hook();
        hudhook::lifecycle::global_state::set_hooks(hooks);
    }

    thread::sleep(Duration::from_millis(5000));
    drop(dx12_harness);
}

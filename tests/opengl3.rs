mod harness;

use std::thread;
use std::time::Duration;

use harness::opengl3::Opengl3Harness;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::hooks::{self, ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::{Condition, Window};
use simplelog::*;

#[test]
fn test_imgui_opengl3() {
    struct Opengl3HookExample;

    impl Opengl3HookExample {
        fn new() -> Self {
            println!("Initializing");
            hudhook::utils::alloc_console();

            Opengl3HookExample
        }
    }

    impl ImguiRenderLoop for Opengl3HookExample {
        fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
            Window::new("Hello world").size([300.0, 300.0], Condition::FirstUseEver).build(
                ui,
                || {
                    ui.text("Hello world!");
                    ui.text("こんにちは世界！");
                    ui.text("This...is...imgui-rs!");
                    ui.separator();
                    let mouse_pos = ui.io().mouse_pos;
                    ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
                },
            );
        }
    }

    TermLogger::init(LevelFilter::Trace, Config::default(), TerminalMode::Mixed, ColorChoice::Auto)
        .ok();

    let opengl3_harness = Opengl3Harness::new("Opengl3 hook example");
    thread::sleep(Duration::from_millis(500));

    unsafe {
        let hooks: Box<dyn hooks::Hooks> =
            { Opengl3HookExample::new().into_hook::<ImguiOpenGl3Hooks>() };
        hooks.hook();
        hudhook::lifecycle::global_state::set_hooks(hooks);
    }

    thread::sleep(Duration::from_millis(5000));
    drop(opengl3_harness);
}

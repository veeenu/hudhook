#![feature(lazy_cell)]

use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::Condition;
struct HookYou;

impl HookYou {
    fn new() -> Self {
        println!("Initializing");
        hudhook::utils::alloc_console();
        #[cfg(feature = "simplelog")]
        hudhook::utils::simplelog();

        HookYou
    }
}

impl ImguiRenderLoop for HookYou {
    fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
        ui.window("Hello world").size([300.0, 110.0], Condition::FirstUseEver).build(|| {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
        });
    }
}

hudhook::hudhook!(HookYou::new().into_hook::<ImguiOpenGl3Hooks>());

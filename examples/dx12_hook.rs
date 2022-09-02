#![feature(once_cell)]

use hudhook::hooks::dx12::ImguiDX12Hooks;
use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::{Condition, Window};
struct Dx12HookExample;

impl Dx12HookExample {
    fn new() -> Self {
        println!("Initializing");
        hudhook::utils::alloc_console();
        hudhook::utils::simplelog();

        Dx12HookExample
    }
}

impl ImguiRenderLoop for Dx12HookExample {
    fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
        Window::new("Hello world").size([300.0, 110.0], Condition::FirstUseEver).build(ui, || {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
        });
    }
}

hudhook::hudhook!(Dx12HookExample::new().into_hook::<ImguiDX12Hooks>());

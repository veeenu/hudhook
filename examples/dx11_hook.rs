#![feature(once_cell)]

use hudhook::hooks::dx11::ImguiDX11Hooks;
use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::{Condition, Window};
struct Dx11HookExample;

impl Dx11HookExample {
    fn new() -> Self {
        println!("Initializing");
        hudhook::utils::alloc_console();
        hudhook::utils::simplelog();

        Dx11HookExample
    }
}

impl ImguiRenderLoop for Dx11HookExample {
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

hudhook::hudhook!(Dx11HookExample::new().into_hook::<ImguiDX11Hooks>());

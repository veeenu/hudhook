#![feature(lazy_cell)]

use hudhook::hooks::dx9::ImguiDx9Hooks;
use hudhook::hooks::ImguiRenderLoop;
use imgui::Condition;
use tracing::metadata::LevelFilter;
struct Dx9HookExample;

impl Dx9HookExample {
    fn new() -> Self {
        println!("Initializing");
        hudhook::alloc_console();
        hudhook::enable_console_colors();

        tracing_subscriber::fmt()
            .with_max_level(LevelFilter::TRACE)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_thread_names(true)
            .init();

        Dx9HookExample
    }
}

impl ImguiRenderLoop for Dx9HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
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

hudhook::hudhook!(Dx9HookExample::new().into_hook::<ImguiDx9Hooks>());

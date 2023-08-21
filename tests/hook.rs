use hudhook::hooks::ImguiRenderLoop;
use imgui::{Condition, StyleColor};

pub struct HookExample;

impl HookExample {
    pub fn new() -> Self {
        println!("Initializing");
        hudhook::alloc_console().ok();

        HookExample
    }
}

impl Default for HookExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ImguiRenderLoop for HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.window("Hello world").size([400.0, 500.0], Condition::FirstUseEver).build(|| {
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

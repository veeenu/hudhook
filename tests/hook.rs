use std::time::{Duration, Instant};

use hudhook::ImguiRenderLoop;
use imgui::{Condition, StyleColor};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_tracing() {
    tracing_subscriber::registry()
        .with(
            fmt::layer().event_format(
                fmt::format()
                    .with_level(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_names(true),
            ),
        )
        .with(EnvFilter::from_default_env())
        .init();
}

pub struct HookExample {
    frame_times: Vec<Duration>,
    last_time: Instant,
}

impl HookExample {
    pub fn new() -> Self {
        println!("Initializing");
        hudhook::alloc_console().ok();

        HookExample { frame_times: Vec::new(), last_time: Instant::now() }
    }
}

impl Default for HookExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ImguiRenderLoop for HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
        let duration = self.last_time.elapsed();
        self.frame_times.push(duration);
        self.last_time = Instant::now();

        let avg: Duration =
            self.frame_times.iter().sum::<Duration>() / self.frame_times.len() as u32;
        let last = self.frame_times.last().unwrap();

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
            ui.text(format!("Frame time: {:8.2}", last.as_secs_f64() * 1000.));
            ui.text(format!("Avg:        {:8.2}", avg.as_secs_f64() * 1000.));
            ui.text(format!("FPS:        {:8.2}", 1. / last.as_secs_f64()));
            ui.text(format!("Avg:        {:8.2}", 1. / avg.as_secs_f64()));
        });
    }
}

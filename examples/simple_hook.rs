use hudhook::ImguiRenderLoop;
use imgui::Condition;
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

pub struct HookExample;

impl ImguiRenderLoop for HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.window("Hello hudhook")
            .size([368.0, 568.0], Condition::FirstUseEver)
            .position([16.0, 16.0], Condition::FirstUseEver)
            .build(|| {
                ui.text("Hello from `hudhook`!");
            });
    }
}

hudhook::hudhook!(hudhook::hooks::dx11::ImguiDx11Hooks, HookExample);

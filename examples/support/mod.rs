use std::fs::File;
use std::sync::Mutex;

use hudhook::*;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_tracing() {
    hudhook::alloc_console().unwrap();
    dotenv::dotenv().ok();
    std::env::set_var("RUST_LOG", "trace");

    let log_file = hudhook::util::get_dll_path()
        .map(|mut path| {
            path.set_extension("log");
            path
        })
        .and_then(|path| File::create(path).ok())
        .unwrap();

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
        .with(
            fmt::layer()
                .with_thread_ids(true)
                .with_file(true)
                .with_line_number(true)
                .with_thread_names(true)
                .with_writer(Mutex::new(log_file))
                .with_ansi(false)
                .boxed(),
        )
        .with(EnvFilter::from_default_env())
        .init();
}

pub struct HookExample(pub bool);

impl ImguiRenderLoop for HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.show_demo_window(&mut self.0);
    }
}

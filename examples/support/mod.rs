use std::fs::File;
use std::io::Cursor;
use std::sync::Mutex;

use hudhook::*;
use image::io::Reader as ImageReader;
use image::{EncodableLayout, RgbaImage};
use imgui::{Condition, Image, TextureId};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_tracing() {
    hudhook::alloc_console().unwrap();
    hudhook::enable_console_colors();
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

pub struct HookExample {
    open: bool,
    image: RgbaImage,
    image_id: Option<TextureId>,
}

impl HookExample {
    pub fn new() -> Self {
        let image = ImageReader::new(Cursor::new(include_bytes!("../../tests/thingken.webp")))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();

        Self { open: true, image, image_id: None }
    }
}

impl ImguiRenderLoop for HookExample {
    fn initialize<'a>(
        &'a mut self,
        _ctx: &mut imgui::Context,
        render_context: &'a mut dyn RenderContext,
    ) {
        let tex_id = render_context
            .load_texture(self.image.as_bytes(), self.image.width(), self.image.height())
            .unwrap();

        self.image_id = Some(tex_id);
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.window("Image")
            .size([192.0, 192.0], Condition::FirstUseEver)
            .position([16.0, 16.0], Condition::FirstUseEver)
            .build(|| {
                Image::new(self.image_id.unwrap(), [
                    self.image.width() as f32,
                    self.image.height() as f32,
                ])
                .build(ui);
            });

        ui.show_demo_window(&mut self.open);
        ui.show_metrics_window(&mut self.open);
    }
}

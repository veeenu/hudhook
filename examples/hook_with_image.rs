use std::io::Cursor;

use hudhook::{ImguiRenderLoop, RenderContext};
use image::io::Reader as ImageReader;
use image::{EncodableLayout, RgbaImage};
use imgui::{Condition, Context, Image, TextureId};
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
    image: RgbaImage,
    image_id: Option<TextureId>,
}

impl HookExample {
    pub fn new() -> Self {
        let image = ImageReader::new(Cursor::new(include_bytes!("../tests/thingken.webp")))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();

        HookExample { image, image_id: None }
    }
}

impl Default for HookExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ImguiRenderLoop for HookExample {
    fn initialize<'a>(&'a mut self, _ctx: &mut Context, render_context: &'a mut dyn RenderContext) {
        self.image_id = render_context
            .load_texture(self.image.as_bytes(), self.image.width() as _, self.image.height() as _)
            .ok();

        println!("{:?}", self.image_id);
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.window("Hello hudhook")
            .size([368.0, 568.0], Condition::FirstUseEver)
            .position([16.0, 16.0], Condition::FirstUseEver)
            .build(|| {
                ui.text("Hello from `hudhook`!");

                if let Some(tex_id) = self.image_id {
                    Image::new(tex_id, [self.image.width() as f32, self.image.height() as f32])
                        .build(ui);
                }
            });
    }
}

hudhook::hudhook!(hudhook::hooks::dx11::ImguiDx11Hooks, HookExample::new());

use std::fs::File;
use std::io::Cursor;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hudhook::{ImguiRenderLoop, TextureLoader};
use image::io::Reader as ImageReader;
use image::{EncodableLayout, RgbaImage};
use imgui::{Condition, Context, Image, StyleColor, TextureId};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_tracing() {
    dotenv::dotenv().ok();

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
    frame_times: Vec<Duration>,
    last_time: Option<Instant>,
    image: RgbaImage,
    image_id: Option<TextureId>,
    image_pos: [f32; 2],
    image_dir: [f32; 2],
}

impl HookExample {
    pub fn new() -> Self {
        println!("Initializing");
        hudhook::alloc_console().ok();

        let image = ImageReader::new(Cursor::new(include_bytes!("thingken.webp")))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();

        HookExample {
            frame_times: Vec::new(),
            last_time: None,
            image,
            image_id: None,
            image_pos: [16.0, 16.0],
            image_dir: [1.0, 1.0],
        }
    }
}

impl Default for HookExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ImguiRenderLoop for HookExample {
    fn initialize<'a>(&'a mut self, _ctx: &mut Context, loader: TextureLoader<'a>) {
        self.image_id =
            loader(self.image.as_bytes(), self.image.width() as _, self.image.height() as _).ok();

        println!("{:?}", self.image_id);
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        if let Some(last_time) = self.last_time.as_mut() {
            let duration = last_time.elapsed();
            self.frame_times.push(duration);
            *last_time = Instant::now();
        } else {
            self.last_time = Some(Instant::now());
        }

        let avg: Duration = if self.frame_times.is_empty() {
            Duration::from_nanos(0)
        } else {
            self.frame_times.iter().sum::<Duration>() / self.frame_times.len() as u32
        };

        let last = self.frame_times.last().copied().unwrap_or_else(|| Duration::from_nanos(0));

        ui.window("Hello world")
            .size([376.0, 568.0], Condition::FirstUseEver)
            .position([16.0, 16.0], Condition::FirstUseEver)
            .build(|| {
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

        ui.window("Image")
            .size([376.0, 568.0], Condition::FirstUseEver)
            .position([408.0, 16.0], Condition::FirstUseEver)
            .build(|| {
                let next_x = self.image_pos[0] + self.image_dir[0];
                let next_y = self.image_pos[1] + self.image_dir[1];

                if next_x <= 16. || next_x >= 376. - 16. - self.image.width() as f32 {
                    self.image_dir[0] = -self.image_dir[0];
                } else {
                    self.image_pos[0] = next_x;
                }
                if next_y <= 16. || next_y >= 568. - 16. - self.image.height() as f32 {
                    self.image_dir[1] = -self.image_dir[1];
                } else {
                    self.image_pos[1] = next_y;
                }

                ui.set_cursor_pos(self.image_pos);

                if let Some(tex_id) = self.image_id {
                    Image::new(tex_id, [self.image.width() as f32, self.image.height() as f32])
                        .build(ui);
                }
            });
    }
}

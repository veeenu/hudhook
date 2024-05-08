use std::fs::File;
use std::io::Cursor;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hudhook::{ImguiRenderLoop, MessageFilter, RenderContext};
use image::imageops::FilterType;
use image::io::Reader as ImageReader;
use image::{DynamicImage, EncodableLayout, RgbaImage};
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

const IMAGE_COUNT: usize = 3;

pub struct HookExample {
    frame_times: Vec<Duration>,
    first_time: Option<Instant>,
    last_time: Option<Instant>,
    dynamic_image: DynamicImage,
    image: [RgbaImage; IMAGE_COUNT],
    image_id: [Option<TextureId>; IMAGE_COUNT],
    image_pos: [[f32; 2]; IMAGE_COUNT],
    image_vel: [[f32; 2]; IMAGE_COUNT],
    last_upload_time: u64,
    main_window_movable: bool,
}

impl HookExample {
    pub fn new() -> Self {
        println!("Initializing");
        hudhook::alloc_console().ok();

        let image0 = ImageReader::new(Cursor::new(include_bytes!("thingken.webp")))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();

        let dynamic_image = DynamicImage::ImageRgba8(image0.clone());
        let image1 = dynamic_image.resize(29, 29, FilterType::Lanczos3).to_rgba8();
        let image2 = dynamic_image.resize(65, 65, FilterType::Lanczos3).to_rgba8();

        HookExample {
            frame_times: Vec::new(),
            first_time: None,
            last_time: None,
            dynamic_image,
            image: [image0, image1, image2],
            image_id: [None, None, None],
            image_pos: [[16.0, 16.0], [16.0, 16.0], [16.0, 16.0]],
            image_vel: [[200.0, 200.0], [100.0, 200.0], [200.0, 100.0]],
            last_upload_time: 0,
            main_window_movable: true,
        }
    }
}

impl Default for HookExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ImguiRenderLoop for HookExample {
    fn initialize<'a>(&'a mut self, _ctx: &mut Context, render_context: &'a mut dyn RenderContext) {
        for i in 0..IMAGE_COUNT {
            let image = &self.image[i];
            self.image_id[i] = render_context
                .load_texture(image.as_bytes(), image.width() as _, image.height() as _)
                .ok();
        }

        println!("{:?}", self.image_id);
    }

    fn before_render<'a>(
        &'a mut self,
        _ctx: &mut Context,
        render_context: &'a mut dyn RenderContext,
    ) {
        let elapsed = self.first_time.get_or_insert(Instant::now()).elapsed().as_secs();
        if elapsed != self.last_upload_time {
            self.last_upload_time = elapsed;

            self.dynamic_image.invert();
            if let Some(texture) = self.image_id[0] {
                render_context
                    .replace_texture(
                        texture,
                        self.dynamic_image.as_bytes(),
                        self.dynamic_image.width(),
                        self.dynamic_image.height(),
                    )
                    .unwrap();
            }
        }
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        let frame_time = if let Some(last_time) = self.last_time.as_mut() {
            let duration = last_time.elapsed();
            self.frame_times.push(duration);
            *last_time = Instant::now();
            duration.as_secs_f32()
        } else {
            self.last_time = Some(Instant::now());
            0.
        };

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
                ui.separator();
                ui.text(format!("Main window movable: {}", self.main_window_movable));
                if ui.button("Toggle movability") {
                    self.main_window_movable ^= true;
                }
            });

        ui.window("Image")
            .size([376.0, 568.0], Condition::FirstUseEver)
            .position([408.0, 16.0], Condition::FirstUseEver)
            .build(|| {
                for i in 0..IMAGE_COUNT {
                    let pos = &mut self.image_pos[i];
                    let vel = &mut self.image_vel[i];
                    let image = &self.image[i];
                    let width = image.width() as f32;
                    let height = image.height() as f32;

                    let next_x = pos[0] + vel[0] * frame_time;
                    let next_y = pos[1] + vel[1] * frame_time;

                    if next_x < 16. || next_x > 376. - 16. - width {
                        vel[0] = -vel[0];
                    }
                    pos[0] = next_x.clamp(16., 376. - 16. - width);

                    if next_y < 16. || next_y > 568. - 16. - height {
                        vel[1] = -vel[1];
                    }
                    pos[1] = next_y.clamp(16., 568. - 16. - height);

                    ui.set_cursor_pos(*pos);

                    if let Some(tex_id) = self.image_id[i] {
                        Image::new(tex_id, [width, height]).build(ui);
                    }
                }
            });
    }

    fn message_filter(&self, _io: &imgui::Io) -> MessageFilter {
        if self.main_window_movable {
            MessageFilter::empty()
        } else {
            MessageFilter::WindowControl
        }
    }
}

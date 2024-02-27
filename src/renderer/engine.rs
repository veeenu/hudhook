use imgui::{DrawData, TextureId};
use windows::core::Result;

pub trait RenderEngine {
    type RenderTarget;

    fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId>;
    fn render(&mut self, draw_data: &DrawData, render_target: Self::RenderTarget) -> Result<()>;
}

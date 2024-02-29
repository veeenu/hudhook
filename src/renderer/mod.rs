//! The [`hudhook`](crate) overlay rendering engine.
mod backend;
mod input;
mod keys;
mod pipeline;

use imgui::{DrawData, TextureId};
use windows::core::Result;

pub(crate) trait RenderEngine {
    type RenderTarget;

    fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId>;
    fn render(&mut self, draw_data: &DrawData, render_target: Self::RenderTarget) -> Result<()>;
}
#[cfg(feature = "dx11")]
pub(crate) use backend::dx11::D3D11RenderEngine;
#[cfg(feature = "dx12")]
pub(crate) use backend::dx12::D3D12RenderEngine;
#[cfg(feature = "dx9")]
pub(crate) use backend::dx9::D3D9RenderEngine;
#[cfg(feature = "opengl3")]
pub(crate) use backend::opengl3::OpenGl3RenderEngine;
pub(crate) use pipeline::Pipeline;

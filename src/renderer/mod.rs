//! The [`hudhook`](crate) overlay rendering engine.
mod backend;
mod input;
mod keys;
pub(crate) mod msg_filter;
mod pipeline;

use imgui::{Context, DrawData};
use windows::core::Result;

use crate::RenderContext;

pub(crate) trait RenderEngine: RenderContext {
    type RenderTarget;

    fn render(&mut self, draw_data: &DrawData, render_target: Self::RenderTarget) -> Result<()>;
    fn setup_fonts(&mut self, ctx: &mut Context) -> Result<()>;
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

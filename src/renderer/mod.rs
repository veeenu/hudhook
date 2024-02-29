//! The [`hudhook`](crate) overlay rendering engine.
mod backend;
mod engine;
mod input;
mod keys;
mod pipeline;

use imgui::TextureId;
use windows::core::Result;

/// A load texture callback. Invoke it in your [`crate::ImguiRenderLoop::initialize`] method for
/// setting up textures.
pub type TextureLoader<'a> = &'a mut dyn FnMut(&'a [u8], u32, u32) -> Result<TextureId>;

pub(crate) use engine::RenderEngine;
pub(crate) use pipeline::Pipeline;

#[cfg(feature = "dx11")]
pub(crate) use backend::dx11::D3D11RenderEngine;
#[cfg(feature = "dx12")]
pub(crate) use backend::dx12::D3D12RenderEngine;
#[cfg(feature = "dx9")]
pub(crate) use backend::dx9::D3D9RenderEngine;
#[cfg(feature = "opengl3")]
pub(crate) use backend::opengl3::OpenGl3RenderEngine;

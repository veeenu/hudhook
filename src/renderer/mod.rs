//! The [`hudhook`](crate) overlay rendering engine.
mod backend;
mod engine;
mod input;
mod keys;
mod pipeline;

pub use engine::RenderEngine;
use imgui::TextureId;
pub use pipeline::Pipeline;
use windows::core::Result;
pub type TextureLoader<'a> = &'a mut dyn FnMut(&'a [u8], u32, u32) -> Result<TextureId>;

#[cfg(feature = "dx12")]
pub use backend::dx12::D3D12RenderEngine;

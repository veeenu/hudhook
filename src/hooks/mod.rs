//! Implementation of platform-specific hooks.
//!
//! Currently DirectX 11 and DirectX 12 hooks with [`imgui`] renderers are
//! available.
//!
//! [`imgui`]: https://docs.rs/imgui/0.8.0/imgui/

pub(crate) mod common;
#[cfg(feature = "dx11")]
pub mod dx11;
#[cfg(feature = "dx12")]
pub mod dx12;
#[cfg(feature = "dx9")]
pub mod dx9;
#[cfg(feature = "opengl3")]
pub mod opengl3;

pub use common::{ImguiRenderLoop, ImguiRenderLoopFlags};

/// Generic trait for platform-specific hooks.
pub trait Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static;

    /// Find the hook target functions addresses, initialize the data, create
    /// and enable the hooks.
    ///
    /// # Safety
    ///
    /// Is most definitely UB.
    unsafe fn hook(&self);

    /// Cleanup global data and disable the hooks.
    ///
    /// # Safety
    ///
    /// Is most definitely UB.
    unsafe fn unhook(&mut self);
}

pub fn initialize() {}

#[inline]
fn loword(l: u32) -> u16 {
    (l & 0xffff) as u16
}
#[inline]
fn hiword(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

#[inline]
fn get_wheel_delta_wparam(wparam: u32) -> u16 {
    hiword(wparam)
}

#[allow(dead_code)]
#[inline]
fn get_xbutton_wparam(wparam: u32) -> u16 {
    hiword(wparam)
}

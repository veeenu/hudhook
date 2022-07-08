//! Implementation of platform-specific hooks.
//!
//! Currently DirectX 11 and DirectX 12 hooks with [`imgui`] renderers are
//! available.
//!
//! [`imgui`]: https://docs.rs/imgui/0.8.0/imgui/

pub(crate) mod common;
pub mod dx11;
pub mod dx12;

/// Generic trait for platform-specific hooks.
pub trait Hooks {
    /// Find the hook target functions addresses, initialize the data, create
    /// and enable the hooks.
    unsafe fn hook(&self);
    /// Cleanup global data and disable the hooks.
    unsafe fn unhook(&mut self);
}

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
    hiword(wparam) as u16
}

#[inline]
fn get_xbutton_wparam(wparam: u32) -> u16 {
    hiword(wparam)
}

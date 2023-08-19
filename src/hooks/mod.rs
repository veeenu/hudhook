//! Implementation of platform-specific hooks.
//!
//! Currently DirectX 11 and DirectX 12 hooks with [`imgui`] renderers are
//! available.
//!
//! [`imgui`]: https://docs.rs/imgui/0.8.0/imgui/

use imgui::{Context, Io, Ui};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

pub use crate::hooks::common::Hooks;

pub mod common;
#[cfg(feature = "dx11")]
pub mod dx11;
#[cfg(feature = "dx12")]
pub mod dx12;
#[cfg(feature = "dx9")]
pub mod dx9;
#[cfg(feature = "opengl3")]
pub mod opengl3;

/// Implement your `imgui` rendering logic via this trait.
pub trait ImguiRenderLoop {
    /// Called once at the first occurrence of the hook. Implement this to
    /// initialize your data.
    fn initialize(&mut self, _ctx: &mut Context) {}

    /// Called every frame. Use the provided `ui` object to build your UI.
    fn render(&mut self, ui: &mut Ui);

    /// Called during the window procedure.
    fn on_wnd_proc(&self, _hwnd: HWND, _umsg: u32, _wparam: WPARAM, _lparam: LPARAM) {}

    /// If this function returns true, the WndProc function will not call the
    /// procedure of the parent window.
    fn should_block_messages(&self, _io: &Io) -> bool {
        false
    }

    fn into_hook<T>(self) -> Box<T>
    where
        T: Hooks,
        Self: Send + Sync + Sized + 'static,
    {
        T::from_render_loop(self)
    }
}

// #[inline]
// fn loword(l: u32) -> u16 {
//     (l & 0xffff) as u16
// }
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

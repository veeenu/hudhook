use crate::{mh::MhHook, ImguiRenderLoop};

pub mod dx11;

/// Generic trait for platform-specific hooks.
///
/// Implement this if you are building a custom renderer.
///
/// Check out first party implementations ([`crate::hooks::dx9`],
/// [`crate::hooks::dx11`], [`crate::hooks::dx12`], [`crate::hooks::opengl3`])
/// for guidance on how to implement the methods.
pub trait Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static;

    /// Return the list of hooks to be enabled, in order.
    fn hooks(&self) -> &[MhHook];

    /// Cleanup global data and disable the hooks.
    ///
    /// # Safety
    ///
    /// Is most definitely UB.
    unsafe fn unhook(&mut self);
}

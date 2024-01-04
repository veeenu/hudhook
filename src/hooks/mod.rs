use crate::mh::MhHook;

/// Generic trait for platform-specific hooks.
///
/// Implement this if you are building a custom renderer.
///
/// Check out first party implementations ([`crate::hooks::dx9`],
/// [`crate::hooks::dx11`], [`crate::hooks::dx12`], [`crate::hooks::opengl3`])
/// for guidance on how to implement the methods.
pub trait Hooks {
    /// Return the list of hooks to be enabled, in order.
    fn hooks(&self) -> &[MhHook];

    /// Cleanup global data and disable the hooks.
    ///
    /// # Safety
    ///
    /// Is most definitely UB.
    unsafe fn unhook(&mut self);
}

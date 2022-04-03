#![allow(dead_code, non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::ptr::null_mut;

use log::*;

pub use winapi::shared::minwindef::LPVOID;
pub use winapi::shared::ntdef::{LPCSTR, LPCWSTR};

#[allow(non_camel_case_types)]
#[must_use]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MH_STATUS {
    /// Unknown error. Should not be returned.
    MH_UNKNOWN = -1,
    /// Successful.
    MH_OK = 0,
    /// MinHook is already initialized.
    MH_ERROR_ALREADY_INITIALIZED,
    /// MinHook is not initialized yet, or already uninitialized.
    MH_ERROR_NOT_INITIALIZED,
    /// The hook for the specified target function is already created.
    MH_ERROR_ALREADY_CREATED,
    /// The hook for the specified target function is not created yet.
    MH_ERROR_NOT_CREATED,
    /// The hook for the specified target function is already enabled.
    MH_ERROR_ENABLED,
    /// The hook for the specified target function is not enabled yet, or
    /// already disabled.
    MH_ERROR_DISABLED,
    /// The specified pointer is invalid. It points the address of non-allocated
    /// and/or non-executable region.
    MH_ERROR_NOT_EXECUTABLE,
    /// The specified target function cannot be hooked.
    MH_ERROR_UNSUPPORTED_FUNCTION,
    /// Failed to allocate memory.
    MH_ERROR_MEMORY_ALLOC,
    /// Failed to change the memory protection.
    MH_ERROR_MEMORY_PROTECT,
    /// The specified module is not loaded.
    MH_ERROR_MODULE_NOT_FOUND,
    /// The specified function is not found.
    MH_ERROR_FUNCTION_NOT_FOUND,
}

extern "system" {
    pub fn MH_Initialize() -> MH_STATUS;
    pub fn MH_Uninitialize() -> MH_STATUS;
    pub fn MH_CreateHook(pTarget: LPVOID, pDetour: LPVOID, ppOriginal: *mut LPVOID) -> MH_STATUS;
    pub fn MH_EnableHook(pTarget: LPVOID) -> MH_STATUS;
    pub fn MH_QueueEnableHook(pTarget: LPVOID) -> MH_STATUS;
    pub fn MH_DisableHook(pTarget: LPVOID) -> MH_STATUS;
    pub fn MH_QueueDisableHook(pTarget: LPVOID) -> MH_STATUS;
    pub fn MH_ApplyQueued() -> MH_STATUS;
}

/// Structure that holds original address, hook function address, and trampoline address
/// for a given hook.
pub struct Hook {
    addr: *mut c_void,
    hook_impl: *mut c_void,
    trampoline: *mut c_void,
}

impl Hook {
    /// # Safety
    ///
    ///
    pub unsafe fn new(addr: *mut c_void, hook_impl: *mut c_void) -> Hook {
        Hook {
            addr,
            hook_impl,
            trampoline: null_mut(),
        }
    }

    pub fn trampoline(&self) -> *mut c_void {
        self.trampoline
    }

    unsafe fn queue_enable(&self) {
        let status = MH_QueueEnableHook(self.hook_impl);
        debug!("MH_QueueEnableHook: {:?}", status);
    }

    unsafe fn queue_disable(&self) {
        let status = MH_QueueDisableHook(self.hook_impl);
        debug!("MH_QueueDisableHook: {:?}", status);
    }
}

/// Wrapper for a queue of hooks to be applied via Minhook.
pub struct Hooks(Vec<Hook>);
unsafe impl Send for Hooks {}
unsafe impl Sync for Hooks {}

impl Hooks {
    pub fn new<F: Fn() -> T, T: IntoIterator<Item = Hook>>(hooks: F) -> Hooks {
        let status = unsafe { MH_Initialize() };
        debug!("MH_Initialize: {:?}", status);

        let hooks = hooks().into_iter().collect::<Vec<_>>();

        unsafe { Hooks::apply_hooks(&hooks) };
        Hooks(hooks)
    }

    pub fn unapply(&self) {
        unsafe { Hooks::unapply_hooks(&self.0) };
        let status = unsafe { MH_Uninitialize() };
        debug!("MH_Uninitialize: {:?}", status);
    }

    unsafe fn apply_hooks(hooks: &[Hook]) {
        for hook in hooks {
            let status = MH_QueueEnableHook(hook.addr);
            debug!("MH_QueueEnable: {:?}", status);
        }
        let status = MH_ApplyQueued();
        debug!("MH_ApplyQueued: {:?}", status);
    }

    unsafe fn unapply_hooks(hooks: &[Hook]) {
        for hook in hooks {
            let status = MH_QueueDisableHook(hook.addr);
            debug!("MH_QueueDisable: {:?}", status);
        }
        let status = MH_ApplyQueued();
        debug!("MH_ApplyQueued: {:?}", status);
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        self.unapply();
    }
}
